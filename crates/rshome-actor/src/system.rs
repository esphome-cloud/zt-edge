use crate::actor::{Actor, ActorError, ActorId};
use crate::actor_ref::ActorRef;
use crate::context::{ActorContext, ChildHandle};
use crate::mailbox::Mailbox;
use crate::supervisor::{check_restart_limit, SupervisorStrategy};
use parking_lot::Mutex;
use std::any::Any;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::task::AbortHandle;

// ── SystemHandle ─────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct SystemHandle {
    inner: Arc<ActorSystemInner>,
}

struct ActorSystemInner {
    tasks: Mutex<HashMap<ActorId, AbortHandle>>,
    shutdown_tx: tokio::sync::broadcast::Sender<()>,
}

impl SystemHandle {
    pub(crate) fn register(&self, id: ActorId, handle: AbortHandle) {
        self.inner.tasks.lock().insert(id, handle);
    }

    pub(crate) fn deregister(&self, id: &ActorId) {
        self.inner.tasks.lock().remove(id);
    }

    #[allow(dead_code)]
    pub(crate) fn shutdown_receiver(&self) -> tokio::sync::broadcast::Receiver<()> {
        self.inner.shutdown_tx.subscribe()
    }
}

// ── ActorSystem ───────────────────────────────────────────────────────────────

#[allow(clippy::module_name_repetitions)]
pub struct ActorSystem {
    handle: SystemHandle,
}

impl ActorSystem {
    pub fn new() -> Self {
        let (shutdown_tx, _) = tokio::sync::broadcast::channel(1);
        let inner = Arc::new(ActorSystemInner {
            tasks: Mutex::new(HashMap::new()),
            shutdown_tx,
        });
        Self {
            handle: SystemHandle { inner },
        }
    }

    pub fn handle(&self) -> SystemHandle {
        self.handle.clone()
    }

    /// Spawn a top-level actor. Returns an `ActorRef`.
    pub fn spawn<A: Actor>(&self, actor: A) -> ActorRef<A::Msg> {
        let children = Arc::new(Mutex::new(Vec::new()));
        spawn_actor_under(
            actor,
            None,
            None,
            SupervisorStrategy::default(),
            self.handle.clone(),
            children,
            None,
            None,
        )
    }

    /// Spawn a top-level actor with a bounded mailbox of the given capacity.
    pub fn spawn_with_capacity<A: Actor>(&self, actor: A, capacity: usize) -> ActorRef<A::Msg> {
        let children = Arc::new(Mutex::new(Vec::new()));
        spawn_actor_under(
            actor,
            None,
            None,
            SupervisorStrategy::default(),
            self.handle.clone(),
            children,
            None,
            Some(capacity),
        )
    }

    /// Spawn a top-level actor with a factory for supervised restarts.
    pub fn spawn_with_factory<A, F>(
        &self,
        factory: F,
        strategy: SupervisorStrategy,
    ) -> ActorRef<A::Msg>
    where
        A: Actor,
        F: Fn() -> A + Send + Sync + 'static,
    {
        let children = Arc::new(Mutex::new(Vec::new()));
        let factory = Arc::new(factory);
        let actor = factory();
        spawn_actor_under_typed(
            actor,
            factory,
            None,
            strategy,
            self.handle.clone(),
            children,
        )
    }

    /// Shut down all actors: broadcast shutdown, abort all registered tasks, then yield so
    /// cancelled futures are actually dropped (and their channel receivers closed).
    pub async fn shutdown(&self) {
        // Signal shutdown; receivers may ignore the error if no listeners
        let _ = self.handle.inner.shutdown_tx.send(());
        // Collect and abort all tasks
        let handles: Vec<AbortHandle> = {
            let mut map = self.handle.inner.tasks.lock();
            map.drain().map(|(_, h)| h).collect()
        };
        for h in &handles {
            h.abort();
        }
        // Yield to allow the scheduler to process the aborts and drop receivers.
        tokio::task::yield_now().await;
    }
}

impl Default for ActorSystem {
    fn default() -> Self {
        Self::new()
    }
}

// ── Core spawn logic ──────────────────────────────────────────────────────────

/// Shared by `ActorSystem::spawn` and `ActorContext::spawn_child*`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn spawn_actor_under<A: Actor>(
    actor: A,
    parent_ref: Option<Box<dyn Any + Send + Sync>>,
    parent_id: Option<ActorId>,
    strategy: SupervisorStrategy,
    system: SystemHandle,
    parent_children: Arc<Mutex<Vec<ChildHandle>>>,
    factory: Option<Arc<dyn Fn() -> Box<dyn Any + Send + Sync> + Send + Sync>>,
    capacity: Option<usize>,
) -> ActorRef<A::Msg> {
    let id = ActorId::new();
    let mailbox = match capacity {
        Some(cap) => Mailbox::<A::Msg>::bounded(cap),
        None => Mailbox::<A::Msg>::unbounded(),
    };
    let actor_ref = ActorRef::new(id.clone(), mailbox.tx.clone());
    let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
    let ctx_children = Arc::new(Mutex::new(Vec::<ChildHandle>::new()));

    let mut ctx = ActorContext::new(actor_ref.clone(), parent_ref, system.clone());
    // Use per-actor children list (owned by context & supervisor monitor)
    ctx.children = ctx_children.clone();

    let system_for_task = system.clone();
    let id_for_task = id.clone();
    let id_for_register = id.clone();

    let join = tokio::spawn(run_actor_loop(
        actor,
        mailbox.rx,
        ctx,
        stop_rx,
        system_for_task,
        id_for_task,
    ));

    let abort = join.abort_handle();
    system.register(id_for_register.clone(), abort);

    // Supervisor monitor — owns join handle
    let system_for_monitor = system.clone();
    let strategy_for_monitor = strategy.clone();
    let parent_children_for_monitor = parent_children.clone();
    let factory_for_monitor = factory.clone();
    let id_for_monitor = id.clone();

    tokio::spawn(supervisor_monitor(
        join,
        id_for_monitor.clone(),
        system_for_monitor,
        strategy_for_monitor,
        parent_children_for_monitor,
        factory_for_monitor,
        parent_id,
    ));

    // Register child handle in parent's children list
    parent_children.lock().push(ChildHandle {
        id: id.clone(),
        stop_tx,
        restart_times: Vec::new(),
        strategy,
        factory,
    });

    actor_ref
}

// ── Actor loop ────────────────────────────────────────────────────────────────

async fn run_actor_loop<A: Actor>(
    mut actor: A,
    rx: flume::Receiver<A::Msg>,
    mut ctx: ActorContext<A::Msg>,
    mut stop_rx: tokio::sync::watch::Receiver<bool>,
    system: SystemHandle,
    id: ActorId,
) {
    actor.pre_start(&mut ctx).await;

    loop {
        tokio::select! {
            msg = rx.recv_async() => match msg {
                Ok(m) => {
                    actor.handle(m, &mut ctx).await;
                    if ctx.stop_requested {
                        break;
                    }
                }
                Err(_) => break, // all ActorRefs dropped
            },
            res = stop_rx.changed() => {
                if res.is_ok() && *stop_rx.borrow() {
                    break;
                }
            }
        }
    }

    // Stop all children
    {
        let children = ctx.children.lock();
        for child in children.iter() {
            let _ = child.stop_tx.send(true);
        }
    }

    actor.post_stop().await;
    system.deregister(&id);
}

// ── Supervisor monitor ────────────────────────────────────────────────────────

async fn supervisor_monitor(
    join: tokio::task::JoinHandle<()>,
    id: ActorId,
    system: SystemHandle,
    strategy: SupervisorStrategy,
    parent_children: Arc<Mutex<Vec<ChildHandle>>>,
    _factory: Option<Arc<dyn Fn() -> Box<dyn Any + Send + Sync> + Send + Sync>>,
    _parent_id: Option<ActorId>,
) {
    let result = join.await;

    match result {
        Ok(()) => {
            // Normal exit — remove from parent children list
            remove_child(&parent_children, &id);
        }
        Err(join_err) if join_err.is_panic() => {
            let panic_msg = join_err
                .into_panic()
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .unwrap_or_else(|| "unknown panic".to_string());
            let err = ActorError::Panicked(panic_msg);

            match &strategy {
                SupervisorStrategy::OneForOne {
                    max_restarts,
                    within_secs,
                } => {
                    let allowed = {
                        let mut children = parent_children.lock();
                        if let Some(ch) = children.iter_mut().find(|c| c.id == id) {
                            check_restart_limit(&mut ch.restart_times, *max_restarts, *within_secs)
                        } else {
                            false
                        }
                    };
                    if allowed {
                        tracing::warn!(actor_id = %id, %err, "actor panicked but no typed factory; not restarting");
                    } else {
                        tracing::warn!(actor_id = %id, "actor restart limit exceeded");
                    }
                    remove_child(&parent_children, &id);
                }
                SupervisorStrategy::RestartN { max, within_secs } => {
                    let allowed = {
                        let mut children = parent_children.lock();
                        if let Some(ch) = children.iter_mut().find(|c| c.id == id) {
                            check_restart_limit(&mut ch.restart_times, *max as u32, *within_secs)
                        } else {
                            false
                        }
                    };
                    if allowed {
                        tracing::warn!(actor_id = %id, %err, "actor panicked but no typed factory; not restarting");
                    } else {
                        tracing::warn!(actor_id = %id, "actor restart limit exceeded");
                    }
                    remove_child(&parent_children, &id);
                }
                SupervisorStrategy::AllForOne { .. } => {
                    // Stop all siblings (best-effort Phase 0)
                    let children = parent_children.lock();
                    for ch in children.iter() {
                        if ch.id != id {
                            let _ = ch.stop_tx.send(true);
                        }
                    }
                    drop(children);
                    remove_child(&parent_children, &id);
                }
                SupervisorStrategy::Escalate => {
                    tracing::warn!(actor_id = %id, %err, "actor panicked; escalating to parent");
                    remove_child(&parent_children, &id);
                }
            }
        }
        Err(_) => {
            // Cancelled (abort) — normal shutdown
            remove_child(&parent_children, &id);
        }
    }

    system.deregister(&id);
}

fn remove_child(children: &Arc<Mutex<Vec<ChildHandle>>>, id: &ActorId) {
    children.lock().retain(|c| &c.id != id);
}

// ── Typed restart path (spawn_child_with_factory / spawn_with_factory) ────────

/// Spawn an actor with a typed factory. The supervisor monitor can call `pre_restart` on new
/// instances and perform actual typed restarts, unlike the type-erased `spawn_actor_under` path.
pub(crate) fn spawn_actor_under_typed<A, F>(
    actor: A,
    factory: Arc<F>,
    _parent_id: Option<ActorId>,
    strategy: SupervisorStrategy,
    system: SystemHandle,
    parent_children: Arc<Mutex<Vec<ChildHandle>>>,
) -> ActorRef<A::Msg>
where
    A: Actor,
    F: Fn() -> A + Send + Sync + 'static,
{
    let id = ActorId::new();
    let mailbox = Mailbox::<A::Msg>::unbounded();
    let actor_ref = ActorRef::new(id.clone(), mailbox.tx);
    let rx = mailbox.rx;
    let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
    let ctx_children = Arc::new(Mutex::new(Vec::<ChildHandle>::new()));

    let mut ctx = ActorContext::new(actor_ref.clone(), None, system.clone());
    ctx.children = ctx_children.clone();

    let system_for_task = system.clone();
    let id_for_task = id.clone();

    // Pass rx.clone() to the loop; the monitor keeps rx as the master for restarts.
    let join = tokio::spawn(run_actor_loop(
        actor,
        rx.clone(),
        ctx,
        stop_rx,
        system_for_task,
        id_for_task,
    ));
    let abort = join.abort_handle();
    system.register(id.clone(), abort);

    tokio::spawn(typed_restart_monitor(
        join,
        id.clone(),
        actor_ref.clone(),
        rx,
        system.clone(),
        strategy.clone(),
        parent_children.clone(),
        factory,
    ));

    parent_children.lock().push(ChildHandle {
        id: id.clone(),
        stop_tx,
        restart_times: Vec::new(),
        strategy,
        factory: None,
    });

    actor_ref
}

#[allow(clippy::too_many_arguments)]
async fn typed_restart_monitor<A, F>(
    mut join: tokio::task::JoinHandle<()>,
    mut current_id: ActorId,
    actor_ref: ActorRef<A::Msg>,
    rx: flume::Receiver<A::Msg>,
    system: SystemHandle,
    strategy: SupervisorStrategy,
    parent_children: Arc<Mutex<Vec<ChildHandle>>>,
    factory: Arc<F>,
) where
    A: Actor,
    F: Fn() -> A + Send + Sync + 'static,
{
    loop {
        let result = join.await;

        match result {
            Ok(()) => {
                remove_child(&parent_children, &current_id);
                system.deregister(&current_id);
                return;
            }
            Err(join_err) if join_err.is_panic() => {
                let panic_msg = join_err
                    .into_panic()
                    .downcast_ref::<&str>()
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| "unknown panic".to_string());
                let err = ActorError::Panicked(panic_msg);

                let allowed = match &strategy {
                    SupervisorStrategy::OneForOne {
                        max_restarts,
                        within_secs,
                    } => {
                        let mut children = parent_children.lock();
                        if let Some(ch) = children.iter_mut().find(|c| c.id == current_id) {
                            check_restart_limit(&mut ch.restart_times, *max_restarts, *within_secs)
                        } else {
                            false
                        }
                    }
                    SupervisorStrategy::RestartN { max, within_secs } => {
                        let mut children = parent_children.lock();
                        if let Some(ch) = children.iter_mut().find(|c| c.id == current_id) {
                            check_restart_limit(&mut ch.restart_times, *max as u32, *within_secs)
                        } else {
                            false
                        }
                    }
                    SupervisorStrategy::AllForOne { .. } => {
                        let children = parent_children.lock();
                        for ch in children.iter() {
                            if ch.id != current_id {
                                let _ = ch.stop_tx.send(true);
                            }
                        }
                        drop(children);
                        remove_child(&parent_children, &current_id);
                        system.deregister(&current_id);
                        return;
                    }
                    SupervisorStrategy::Escalate => {
                        tracing::warn!(actor_id = %current_id, %err, "actor panicked; escalating to parent");
                        remove_child(&parent_children, &current_id);
                        system.deregister(&current_id);
                        return;
                    }
                };

                if allowed {
                    // Create new actor instance from factory and call pre_restart.
                    let mut new_actor = factory();
                    new_actor.pre_restart(&err).await;

                    let new_id = ActorId::new();
                    let (stop_tx_new, stop_rx_new) = tokio::sync::watch::channel(false);
                    let ctx_children_new = Arc::new(Mutex::new(Vec::<ChildHandle>::new()));
                    let mut ctx_new = ActorContext::new(actor_ref.clone(), None, system.clone());
                    ctx_new.children = ctx_children_new;

                    let new_id_for_loop = new_id.clone();
                    let new_join = tokio::spawn(run_actor_loop(
                        new_actor,
                        rx.clone(),
                        ctx_new,
                        stop_rx_new,
                        system.clone(),
                        new_id_for_loop,
                    ));
                    let abort = new_join.abort_handle();
                    system.register(new_id.clone(), abort);

                    // Update the child handle to point at the new actor.
                    {
                        let mut children = parent_children.lock();
                        if let Some(ch) = children.iter_mut().find(|c| c.id == current_id) {
                            ch.id = new_id.clone();
                            ch.stop_tx = stop_tx_new;
                        }
                    }

                    system.deregister(&current_id);
                    tracing::debug!(old_id = %current_id, new_id = %new_id, "actor restarted");
                    current_id = new_id;
                    join = new_join;
                    // Continue loop to monitor the new actor instance.
                } else {
                    tracing::warn!(actor_id = %current_id, %err, "actor restart limit exceeded");
                    remove_child(&parent_children, &current_id);
                    system.deregister(&current_id);
                    return;
                }
            }
            Err(_) => {
                // Aborted (normal shutdown path).
                remove_child(&parent_children, &current_id);
                system.deregister(&current_id);
                return;
            }
        }
    }
}
