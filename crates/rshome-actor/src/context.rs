use crate::actor::{Actor, ActorId};
use crate::actor_ref::ActorRef;
use crate::supervisor::SupervisorStrategy;
use crate::system::SystemHandle;
use std::any::Any;
use std::sync::Arc;

pub(crate) struct ChildHandle {
    pub(crate) id: ActorId,
    pub(crate) stop_tx: tokio::sync::watch::Sender<bool>,
    pub(crate) restart_times: Vec<tokio::time::Instant>,
    #[allow(dead_code)]
    pub(crate) strategy: SupervisorStrategy,
    #[allow(dead_code)]
    pub(crate) factory: Option<Arc<dyn Fn() -> Box<dyn Any + Send + Sync> + Send + Sync>>,
}

#[allow(clippy::module_name_repetitions)]
pub struct ActorContext<M: Send + 'static> {
    pub(crate) self_ref: ActorRef<M>,
    pub(crate) parent_ref: Option<Box<dyn Any + Send + Sync>>,
    pub(crate) system_handle: SystemHandle,
    pub(crate) children: Arc<parking_lot::Mutex<Vec<ChildHandle>>>,
    pub(crate) stop_requested: bool,
}

impl<M: Send + 'static> ActorContext<M> {
    pub(crate) fn new(
        self_ref: ActorRef<M>,
        parent_ref: Option<Box<dyn Any + Send + Sync>>,
        system_handle: SystemHandle,
    ) -> Self {
        Self {
            self_ref,
            parent_ref,
            system_handle,
            children: Arc::new(parking_lot::Mutex::new(Vec::new())),
            stop_requested: false,
        }
    }

    pub fn self_ref(&self) -> &ActorRef<M> {
        &self.self_ref
    }

    pub fn parent_ref(&self) -> Option<&(dyn Any + Send + Sync)> {
        self.parent_ref.as_deref()
    }

    /// Request this actor to stop after the current message finishes.
    pub fn stop(&mut self) {
        self.stop_requested = true;
    }

    /// Spawn a child actor with a given strategy. Returns an `ActorRef` to the child.
    pub fn spawn_child<A: Actor>(
        &mut self,
        actor: A,
        strategy: SupervisorStrategy,
    ) -> ActorRef<A::Msg> {
        crate::system::spawn_actor_under(
            actor,
            None,
            Some(self.self_ref.id.clone()),
            strategy,
            self.system_handle.clone(),
            self.children.clone(),
            None,
            None,
        )
    }

    /// Spawn a child with a factory closure for supervised restarts.
    /// Uses the typed restart path: `pre_restart` is called on each new instance before `pre_start`.
    pub fn spawn_child_with_factory<A, F>(
        &mut self,
        factory: F,
        strategy: SupervisorStrategy,
    ) -> ActorRef<A::Msg>
    where
        A: Actor,
        F: Fn() -> A + Send + Sync + 'static,
    {
        let factory = Arc::new(factory);
        let actor = factory();
        crate::system::spawn_actor_under_typed(
            actor,
            factory,
            Some(self.self_ref.id.clone()),
            strategy,
            self.system_handle.clone(),
            self.children.clone(),
        )
    }

    /// Spawn a child with a bounded mailbox of the given capacity.
    pub fn spawn_child_with_capacity<A: Actor>(
        &mut self,
        actor: A,
        capacity: usize,
        strategy: SupervisorStrategy,
    ) -> ActorRef<A::Msg> {
        crate::system::spawn_actor_under(
            actor,
            None,
            Some(self.self_ref.id.clone()),
            strategy,
            self.system_handle.clone(),
            self.children.clone(),
            None,
            Some(capacity),
        )
    }

    /// Spawn a child with default `OneForOne` strategy (no factory = no restart on panic).
    pub fn spawn_child_default<A: Actor>(&mut self, actor: A) -> ActorRef<A::Msg> {
        self.spawn_child(actor, SupervisorStrategy::default())
    }

    pub fn system(&self) -> &SystemHandle {
        &self.system_handle
    }
}
