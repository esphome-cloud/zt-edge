use crate::codec::EspHomeCodec;
use crate::command_dispatch::CommandDispatcher;
use crate::entity_list::build_entity_list;
use crate::msg_types;
use crate::proto_gen::*;
use crate::state_push::state_to_frame;
use futures::{SinkExt, StreamExt as _};
use prost::Message;
use rshome_actor::{Actor, ActorContext, ActorRef};
use rshome_entity::{EntityId, EntityRegistry, EntityState};
use rshome_state::StateStore;
use std::sync::Arc;
use std::time::Instant;
use tokio::net::TcpStream;
use tokio::task::JoinHandle;
use tokio_util::codec::{FramedRead, FramedWrite};

#[allow(clippy::module_name_repetitions)]
pub enum ConnectionMsg {
    InboundFrame { msg_type: u32, payload: Vec<u8> },
    StateUpdate(EntityId, EntityState),
    KeepaliveTick,
    ReaderClosed,
}

enum ConnState {
    AwaitingHello,
    Active { subscribed: bool },
    Disconnecting,
}

#[allow(clippy::module_name_repetitions)]
pub struct ConnectionActor {
    state: ConnState,
    writer: FramedWrite<tokio::net::tcp::OwnedWriteHalf, EspHomeCodec>,
    reader: Option<tokio::net::tcp::OwnedReadHalf>,
    registry: EntityRegistry,
    state_store: Arc<StateStore>,
    dispatcher: CommandDispatcher,
    device_name: String,
    last_activity: Instant,
    watcher_handles: Vec<JoinHandle<()>>,
}

impl ConnectionActor {
    pub fn new(
        stream: TcpStream,
        registry: EntityRegistry,
        state_store: Arc<StateStore>,
        dispatcher: CommandDispatcher,
        device_name: String,
    ) -> Self {
        let (read_half, write_half) = stream.into_split();
        Self {
            state: ConnState::AwaitingHello,
            writer: FramedWrite::new(write_half, EspHomeCodec),
            reader: Some(read_half),
            registry,
            state_store,
            dispatcher,
            device_name,
            last_activity: Instant::now(),
            watcher_handles: Vec::new(),
        }
    }

    async fn send_frame(&mut self, msg_type: u32, payload: Vec<u8>) {
        let _ = self.writer.send((msg_type, payload)).await;
    }
}

#[async_trait::async_trait]
impl Actor for ConnectionActor {
    type Msg = ConnectionMsg;

    async fn pre_start(&mut self, ctx: &mut ActorContext<Self::Msg>) {
        // Move the reader half into a background reader task
        if let Some(reader) = self.reader.take() {
            let self_ref = ctx.self_ref().clone();
            let framed = FramedRead::new(reader, EspHomeCodec);
            tokio::spawn(async move {
                let mut framed = framed;
                loop {
                    match framed.next().await {
                        Some(Ok((msg_type, payload))) => {
                            if self_ref
                                .send(ConnectionMsg::InboundFrame { msg_type, payload })
                                .is_err()
                            {
                                break;
                            }
                        }
                        Some(Err(_)) | None => {
                            let _ = self_ref.send(ConnectionMsg::ReaderClosed);
                            break;
                        }
                    }
                }
            });
        }

        // Keepalive ticker
        let self_ref = ctx.self_ref().clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(5));
            loop {
                interval.tick().await;
                if self_ref.send(ConnectionMsg::KeepaliveTick).is_err() {
                    break;
                }
            }
        });
    }

    async fn handle(&mut self, msg: ConnectionMsg, ctx: &mut ActorContext<Self::Msg>) {
        match msg {
            ConnectionMsg::InboundFrame { msg_type, payload } => {
                self.last_activity = Instant::now();
                self.handle_frame(msg_type, payload, ctx).await;
            }
            ConnectionMsg::StateUpdate(id, state) => {
                if let ConnState::Active { subscribed: true } = &self.state {
                    if let Some(frame) = state_to_frame(&id, &state) {
                        self.send_frame(frame.0, frame.1).await;
                    }
                }
            }
            ConnectionMsg::KeepaliveTick => {
                if self.last_activity.elapsed() > std::time::Duration::from_secs(30) {
                    ctx.stop();
                }
            }
            ConnectionMsg::ReaderClosed => {
                ctx.stop();
            }
        }
    }

    async fn post_stop(&mut self) {
        for handle in &self.watcher_handles {
            handle.abort();
        }
    }
}

impl ConnectionActor {
    async fn handle_frame(
        &mut self,
        msg_type: u32,
        payload: Vec<u8>,
        ctx: &mut ActorContext<ConnectionMsg>,
    ) {
        match (&self.state, msg_type) {
            (ConnState::AwaitingHello, t) if t == msg_types::HELLO_REQUEST => {
                // Send HelloResponse
                let response = HelloResponse {
                    api_version_major: 1,
                    api_version_minor: 10,
                    server_info: format!("rshome {}", env!("CARGO_PKG_VERSION")),
                    name: self.device_name.clone(),
                };
                self.send_frame(msg_types::HELLO_RESPONSE, response.encode_to_vec())
                    .await;
                self.state = ConnState::Active { subscribed: false };
            }
            (ConnState::AwaitingHello, _) => {
                // Frame before hello — close connection
                ctx.stop();
            }
            (ConnState::Active { .. }, t) if t == msg_types::PING_REQUEST => {
                self.send_frame(msg_types::PING_RESPONSE, PingResponse {}.encode_to_vec())
                    .await;
            }
            (ConnState::Active { .. }, t) if t == msg_types::DEVICE_INFO_REQUEST => {
                let response = DeviceInfoResponse {
                    uses_password: false,
                    name: self.device_name.clone(),
                    esphome_version: "2025.1.0".to_string(),
                    friendly_name: self.device_name.clone(),
                    ..Default::default()
                };
                self.send_frame(msg_types::DEVICE_INFO_RESPONSE, response.encode_to_vec())
                    .await;
            }
            (ConnState::Active { .. }, t) if t == msg_types::LIST_ENTITIES_REQUEST => {
                let frames = build_entity_list(&self.registry).await;
                for (mtype, data) in frames {
                    self.send_frame(mtype, data).await;
                }
                self.send_frame(
                    msg_types::LIST_ENTITIES_DONE,
                    ListEntitiesDoneResponse {}.encode_to_vec(),
                )
                .await;
            }
            (ConnState::Active { .. }, t) if t == msg_types::SUBSCRIBE_STATES => {
                self.state = ConnState::Active { subscribed: true };
                let ids = {
                    let mut ids = self.registry.list_all();
                    ids.sort_by(|a, b| a.0.cmp(&b.0));
                    ids
                };
                let self_ref = ctx.self_ref().clone();
                for id in ids {
                    // Push current state
                    if let Some(state) = self.state_store.get(&id) {
                        if let Some(frame) = state_to_frame(&id, &state) {
                            self.send_frame(frame.0, frame.1).await;
                        }
                    }
                    // Spawn watcher
                    let handle = spawn_watcher(id, self.state_store.clone(), self_ref.clone());
                    self.watcher_handles.push(handle);
                }
            }
            (ConnState::Active { .. }, t) if t == msg_types::DISCONNECT_REQUEST => {
                self.send_frame(
                    msg_types::DISCONNECT_RESPONSE,
                    DisconnectResponse {}.encode_to_vec(),
                )
                .await;
                self.state = ConnState::Disconnecting;
                ctx.stop();
            }
            (ConnState::Active { .. }, t)
                if matches!(
                    t,
                    msg_types::SWITCH_COMMAND
                        | msg_types::LIGHT_COMMAND
                        | msg_types::CLIMATE_COMMAND
                        | msg_types::FAN_COMMAND
                        | msg_types::COVER_COMMAND
                        | msg_types::NUMBER_COMMAND
                        | msg_types::SELECT_COMMAND
                        | msg_types::BUTTON_COMMAND
                ) =>
            {
                if let Err(e) = self.dispatcher.dispatch(t, &payload).await {
                    tracing::warn!("command dispatch error: {e}");
                }
            }
            _ => {
                // Unknown or unexpected frame — ignore
            }
        }
    }
}

fn spawn_watcher(
    entity_id: EntityId,
    state_store: Arc<StateStore>,
    actor_ref: ActorRef<ConnectionMsg>,
) -> JoinHandle<()> {
    let id_clone = entity_id.clone();
    tokio::spawn(async move {
        let mut rx = state_store.subscribe(&entity_id);
        while let Ok(()) = rx.changed().await {
            let state_opt = rx.borrow().clone();
            if let Some(state) = state_opt {
                if actor_ref
                    .send(ConnectionMsg::StateUpdate(id_clone.clone(), state))
                    .is_err()
                {
                    break;
                }
            }
        }
    })
}
