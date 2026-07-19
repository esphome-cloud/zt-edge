use crate::command_dispatch::CommandDispatcher;
use crate::connection::{ConnectionActor, ConnectionMsg};
use crate::mdns::MdnsHandle;
use rshome_actor::{Actor, ActorContext, ActorRef, SupervisorStrategy};
use rshome_entity::EntityRegistry;
use rshome_state::StateStore;
use rshome_svc::ServiceMsg;
use std::sync::Arc;
use tokio::net::TcpListener;

const MAX_CONNECTIONS: usize = 3;

#[allow(clippy::module_name_repetitions)]
pub enum NativeApiMsg {
    Start,
    NewConnection(tokio::net::TcpStream),
    Stop,
}

#[allow(clippy::module_name_repetitions)]
pub struct NativeApiServerActor {
    pub port: u16,
    pub device_name: String,
    pub registry: EntityRegistry,
    pub state_store: Arc<StateStore>,
    pub service_registry: ActorRef<ServiceMsg>,
    connections: Vec<Option<ActorRef<ConnectionMsg>>>,
    mdns: Option<MdnsHandle>,
}

impl NativeApiServerActor {
    pub fn new(
        port: u16,
        device_name: String,
        registry: EntityRegistry,
        state_store: Arc<StateStore>,
        service_registry: ActorRef<ServiceMsg>,
    ) -> Self {
        Self {
            port,
            device_name,
            registry,
            state_store,
            service_registry,
            connections: vec![None; MAX_CONNECTIONS],
            mdns: None,
        }
    }
}

#[async_trait::async_trait]
impl Actor for NativeApiServerActor {
    type Msg = NativeApiMsg;

    async fn handle(&mut self, msg: NativeApiMsg, ctx: &mut ActorContext<NativeApiMsg>) {
        match msg {
            NativeApiMsg::Start => {
                let port = self.port;
                let listener: TcpListener = match TcpListener::bind(("0.0.0.0", port)).await {
                    Ok(l) => l,
                    Err(e) => {
                        tracing::error!("failed to bind TCP listener on port {port}: {e}");
                        ctx.stop();
                        return;
                    }
                };

                tracing::info!("ESPHome Native API listening on port {port}");

                let self_ref = ctx.self_ref().clone();
                tokio::spawn(async move {
                    loop {
                        match listener.accept().await {
                            Ok((stream, addr)) => {
                                tracing::debug!("new connection from {addr}");
                                if self_ref.send(NativeApiMsg::NewConnection(stream)).is_err() {
                                    break;
                                }
                            }
                            Err(e) => {
                                tracing::warn!("accept error: {e}");
                            }
                        }
                    }
                });

                // Register mDNS service
                match MdnsHandle::register(&self.device_name, port) {
                    Ok(handle) => self.mdns = Some(handle),
                    Err(e) => tracing::warn!("mDNS registration failed: {e}"),
                }
            }
            NativeApiMsg::NewConnection(stream) => {
                // Count active connections
                let active = self.connections.iter().filter(|s| s.is_some()).count();
                if active >= MAX_CONNECTIONS {
                    tracing::debug!("max connections reached, dropping new connection");
                    drop(stream);
                    return;
                }

                // Find an empty slot
                if let Some(slot) = self.connections.iter_mut().find(|s| s.is_none()) {
                    let dispatcher = CommandDispatcher {
                        registry: self.registry.clone(),
                        service_registry: self.service_registry.clone(),
                    };
                    let conn = ConnectionActor::new(
                        stream,
                        self.registry.clone(),
                        self.state_store.clone(),
                        dispatcher,
                        self.device_name.clone(),
                    );
                    let conn_ref = ctx.spawn_child(conn, SupervisorStrategy::default());
                    *slot = Some(conn_ref);
                }
            }
            NativeApiMsg::Stop => {
                // Drop all connection refs (actors will be stopped by supervisor)
                for slot in &mut self.connections {
                    *slot = None;
                }
                self.mdns = None;
                ctx.stop();
            }
        }
    }
}
