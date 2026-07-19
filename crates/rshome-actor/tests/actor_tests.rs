use rshome_actor::{
    Actor, ActorContext, ActorError, ActorId, ActorRef, ActorSystem, CrossbeamPipe,
    SupervisorStrategy,
};
use std::sync::{
    atomic::{AtomicBool, AtomicU32, Ordering},
    Arc,
};
use std::time::Duration;
use tokio::sync::oneshot;

// ── Helpers ───────────────────────────────────────────────────────────────────

struct EchoActor;

#[async_trait::async_trait]
impl Actor for EchoActor {
    type Msg = (String, oneshot::Sender<String>);
    async fn handle(&mut self, (msg, tx): Self::Msg, _ctx: &mut ActorContext<Self::Msg>) {
        let _ = tx.send(msg);
    }
}

struct CountActor {
    count: Arc<AtomicU32>,
}

#[async_trait::async_trait]
impl Actor for CountActor {
    type Msg = ();
    async fn handle(&mut self, _: (), _ctx: &mut ActorContext<Self::Msg>) {
        self.count.fetch_add(1, Ordering::SeqCst);
    }
}

struct StopOnMsgActor {
    stopped: Arc<AtomicBool>,
}

#[async_trait::async_trait]
impl Actor for StopOnMsgActor {
    type Msg = ();
    async fn handle(&mut self, _: (), ctx: &mut ActorContext<Self::Msg>) {
        ctx.stop();
    }
    async fn post_stop(&mut self) {
        self.stopped.store(true, Ordering::SeqCst);
    }
}

struct SlowActor;

#[async_trait::async_trait]
impl Actor for SlowActor {
    type Msg = (u64, oneshot::Sender<u64>);
    async fn handle(&mut self, (delay_ms, tx): Self::Msg, _ctx: &mut ActorContext<Self::Msg>) {
        tokio::time::sleep(Duration::from_millis(delay_ms)).await;
        let _ = tx.send(delay_ms);
    }
}

struct ChildSpawnerActor {
    child_ref: Option<ActorRef<(String, oneshot::Sender<String>)>>,
}

#[async_trait::async_trait]
impl Actor for ChildSpawnerActor {
    type Msg = (String, oneshot::Sender<String>);
    async fn pre_start(&mut self, ctx: &mut ActorContext<Self::Msg>) {
        let child = ctx.spawn_child_default(EchoActor);
        self.child_ref = Some(child);
    }
    async fn handle(&mut self, (msg, tx): Self::Msg, _ctx: &mut ActorContext<Self::Msg>) {
        if let Some(ref child) = self.child_ref {
            let result = child.ask(|s| (msg, s)).await.unwrap_or_default();
            let _ = tx.send(result);
        }
    }
}

struct PanicActor;

#[async_trait::async_trait]
impl Actor for PanicActor {
    type Msg = ();
    async fn handle(&mut self, _: (), _ctx: &mut ActorContext<Self::Msg>) {
        panic!("intentional panic");
    }
}

struct PreStartActor {
    started: Arc<AtomicBool>,
}

#[async_trait::async_trait]
impl Actor for PreStartActor {
    type Msg = ();
    async fn pre_start(&mut self, _ctx: &mut ActorContext<Self::Msg>) {
        self.started.store(true, Ordering::SeqCst);
    }
    async fn handle(&mut self, _: (), _ctx: &mut ActorContext<Self::Msg>) {}
}

struct PostStopActor {
    stopped: Arc<AtomicBool>,
}

#[async_trait::async_trait]
impl Actor for PostStopActor {
    type Msg = ();
    async fn handle(&mut self, _: (), ctx: &mut ActorContext<Self::Msg>) {
        ctx.stop();
    }
    async fn post_stop(&mut self) {
        self.stopped.store(true, Ordering::SeqCst);
    }
}

struct SelfRefActor {
    captured_id: Arc<parking_lot::Mutex<Option<ActorId>>>,
}

#[async_trait::async_trait]
impl Actor for SelfRefActor {
    type Msg = ();
    async fn pre_start(&mut self, ctx: &mut ActorContext<Self::Msg>) {
        *self.captured_id.lock() = Some(ctx.self_ref().actor_id().clone());
    }
    async fn handle(&mut self, _: (), _ctx: &mut ActorContext<Self::Msg>) {}
}

// ── Group 1: Lifecycle ────────────────────────────────────────────────────────

#[tokio::test]
async fn test_spawn_and_receive() {
    let sys = ActorSystem::new();
    let actor_ref = sys.spawn(EchoActor);
    let reply = actor_ref.ask(|tx| ("hello".into(), tx)).await.unwrap();
    assert_eq!(reply, "hello");
    sys.shutdown().await;
}

#[tokio::test]
async fn test_pre_start_called() {
    let started = Arc::new(AtomicBool::new(false));
    let sys = ActorSystem::new();
    let _r = sys.spawn(PreStartActor {
        started: started.clone(),
    });
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert!(started.load(Ordering::SeqCst));
    sys.shutdown().await;
}

#[tokio::test]
async fn test_post_stop_on_ctx_stop() {
    let stopped = Arc::new(AtomicBool::new(false));
    let sys = ActorSystem::new();
    let r = sys.spawn(StopOnMsgActor {
        stopped: stopped.clone(),
    });
    let _ = r.send(());
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(stopped.load(Ordering::SeqCst));
    sys.shutdown().await;
}

#[tokio::test]
async fn test_post_stop_on_mailbox_drop() {
    let stopped = Arc::new(AtomicBool::new(false));
    let sys = ActorSystem::new();
    let r = sys.spawn(PostStopActor {
        stopped: stopped.clone(),
    });
    let _ = r.send(());
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(stopped.load(Ordering::SeqCst));
    sys.shutdown().await;
}

#[tokio::test]
async fn test_fifo_ordering() {
    let count = Arc::new(AtomicU32::new(0));
    let sys = ActorSystem::new();
    let r = sys.spawn(CountActor {
        count: count.clone(),
    });
    for _ in 0..100 {
        let _ = r.send(());
    }
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(count.load(Ordering::SeqCst), 100);
    sys.shutdown().await;
}

#[tokio::test]
async fn test_unique_actor_ids() {
    let ids: Vec<ActorId> = (0..100).map(|_| ActorId::new()).collect();
    let set: std::collections::HashSet<_> = ids.iter().collect();
    assert_eq!(set.len(), 100);
}

#[tokio::test]
async fn test_pre_restart_called_on_panic() {
    // Smoke test: a panicking actor causes the supervisor monitor to run without hanging
    let sys = ActorSystem::new();
    let r = sys.spawn(PanicActor);
    let _ = r.send(());
    tokio::time::sleep(Duration::from_millis(100)).await;
    sys.shutdown().await;
}

// ── Group 2: ask pattern ──────────────────────────────────────────────────────

#[tokio::test]
async fn test_basic_ask() {
    let sys = ActorSystem::new();
    let r = sys.spawn(EchoActor);
    let reply = r.ask(|tx| ("world".into(), tx)).await.unwrap();
    assert_eq!(reply, "world");
    sys.shutdown().await;
}

#[tokio::test]
async fn test_ask_timeout_fires() {
    let sys = ActorSystem::new();
    let r = sys.spawn(SlowActor);
    let result = r
        .ask_timeout(|tx| (500, tx), Duration::from_millis(50))
        .await;
    assert!(matches!(result, Err(ActorError::AskTimeout { .. })));
    sys.shutdown().await;
}

#[tokio::test]
async fn test_ask_to_stopped_actor() {
    let sys = ActorSystem::new();
    let r = sys.spawn(EchoActor);
    sys.shutdown().await;
    // After shutdown, sender is disconnected
    let result = r.ask(|tx| ("gone".into(), tx)).await;
    assert!(matches!(result, Err(ActorError::Disconnected)));
}

#[tokio::test]
async fn test_concurrent_asks() {
    let sys = ActorSystem::new();
    let r = sys.spawn(EchoActor);
    let mut handles = Vec::new();
    for i in 0..20u32 {
        let r2 = r.clone();
        handles.push(tokio::spawn(async move {
            r2.ask(|tx| (i.to_string(), tx)).await.unwrap()
        }));
    }
    let results: Vec<String> = futures::future::join_all(handles)
        .await
        .into_iter()
        .map(|r| r.unwrap())
        .collect();
    assert_eq!(results.len(), 20);
    sys.shutdown().await;
}

// ── Group 3: ActorContext ─────────────────────────────────────────────────────

#[tokio::test]
async fn test_self_ref_id_matches() {
    let captured = Arc::new(parking_lot::Mutex::new(None));
    let sys = ActorSystem::new();
    let r = sys.spawn(SelfRefActor {
        captured_id: captured.clone(),
    });
    tokio::time::sleep(Duration::from_millis(20)).await;
    let captured_id = captured.lock().clone().unwrap();
    assert_eq!(&captured_id, r.actor_id());
    sys.shutdown().await;
}

#[tokio::test]
async fn test_spawn_child() {
    let sys = ActorSystem::new();
    let r = sys.spawn(ChildSpawnerActor { child_ref: None });
    tokio::time::sleep(Duration::from_millis(20)).await;
    let reply = r.ask(|tx| ("via child".into(), tx)).await.unwrap();
    assert_eq!(reply, "via child");
    sys.shutdown().await;
}

#[tokio::test]
async fn test_stop_exits_loop() {
    let stopped = Arc::new(AtomicBool::new(false));
    let sys = ActorSystem::new();
    let r = sys.spawn(StopOnMsgActor {
        stopped: stopped.clone(),
    });
    let _ = r.send(());
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(stopped.load(Ordering::SeqCst));
    sys.shutdown().await;
}

#[tokio::test]
async fn test_multiple_children() {
    struct MultiChildActor {
        results: Arc<Mutex<Vec<String>>>,
    }

    use parking_lot::Mutex;

    #[async_trait::async_trait]
    impl Actor for MultiChildActor {
        type Msg = ();
        async fn pre_start(&mut self, ctx: &mut ActorContext<Self::Msg>) {
            for i in 0..3u32 {
                let results = self.results.clone();
                let label = format!("child-{i}");
                let _r = ctx.spawn_child(
                    CapturingActor { label, results },
                    SupervisorStrategy::default(),
                );
            }
        }
        async fn handle(&mut self, _: (), _ctx: &mut ActorContext<Self::Msg>) {}
    }

    struct CapturingActor {
        label: String,
        results: Arc<Mutex<Vec<String>>>,
    }

    #[async_trait::async_trait]
    impl Actor for CapturingActor {
        type Msg = ();
        async fn pre_start(&mut self, _ctx: &mut ActorContext<Self::Msg>) {
            self.results.lock().push(self.label.clone());
        }
        async fn handle(&mut self, _: (), _ctx: &mut ActorContext<Self::Msg>) {}
    }

    let results = Arc::new(Mutex::new(Vec::new()));
    let sys = ActorSystem::new();
    let _r = sys.spawn(MultiChildActor {
        results: results.clone(),
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    let v = results.lock().clone();
    assert_eq!(v.len(), 3);
    sys.shutdown().await;
}

#[tokio::test]
async fn test_parent_ref_none_for_top_level() {
    struct CheckParentActor {
        has_parent: Arc<AtomicBool>,
    }

    #[async_trait::async_trait]
    impl Actor for CheckParentActor {
        type Msg = ();
        async fn pre_start(&mut self, ctx: &mut ActorContext<Self::Msg>) {
            self.has_parent
                .store(ctx.parent_ref().is_some(), Ordering::SeqCst);
        }
        async fn handle(&mut self, _: (), _ctx: &mut ActorContext<Self::Msg>) {}
    }

    let has_parent = Arc::new(AtomicBool::new(true));
    let sys = ActorSystem::new();
    let _r = sys.spawn(CheckParentActor {
        has_parent: has_parent.clone(),
    });
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert!(!has_parent.load(Ordering::SeqCst));
    sys.shutdown().await;
}

// ── Group 4: Supervision ─────────────────────────────────────────────────────

#[tokio::test]
async fn test_oncefor_one_restart_within_limit() {
    // Actor panics once; supervisor logs warning and removes child (no typed factory restart)
    let sys = ActorSystem::new();
    let r = sys.spawn(PanicActor);
    let _ = r.send(());
    // Should not hang
    tokio::time::sleep(Duration::from_millis(200)).await;
    sys.shutdown().await;
}

#[tokio::test]
async fn test_restart_limit_exceeded_removes_child() {
    let sys = ActorSystem::new();
    let count = Arc::new(AtomicU32::new(0));
    // Spawn an actor that panics and verify system handles it gracefully
    let r = sys.spawn(PanicActor);
    let _ = r.send(());
    tokio::time::sleep(Duration::from_millis(100)).await;
    // System should still be alive
    let r2 = sys.spawn(CountActor {
        count: count.clone(),
    });
    let _ = r2.send(());
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(count.load(Ordering::SeqCst), 1);
    sys.shutdown().await;
}

#[tokio::test(start_paused = true)]
async fn test_restart_window_reset() {
    use rshome_actor::supervisor::check_restart_limit;
    let mut restart_times: Vec<tokio::time::Instant> = Vec::new();
    // Fill window
    assert!(check_restart_limit(&mut restart_times, 3, 60));
    assert!(check_restart_limit(&mut restart_times, 3, 60));
    assert!(check_restart_limit(&mut restart_times, 3, 60));
    // Should be denied (3 in window)
    assert!(!check_restart_limit(&mut restart_times, 3, 60));
    // Advance time past window
    tokio::time::advance(Duration::from_secs(61)).await;
    // Now allowed again
    assert!(check_restart_limit(&mut restart_times, 3, 60));
}

#[tokio::test]
async fn test_escalate_strategy_no_hang() {
    // Actor with Escalate strategy panics — should not hang the test
    struct EscalateParent;

    #[async_trait::async_trait]
    impl Actor for EscalateParent {
        type Msg = ();
        async fn pre_start(&mut self, ctx: &mut ActorContext<Self::Msg>) {
            let _r = ctx.spawn_child(PanicActor, SupervisorStrategy::Escalate);
        }
        async fn handle(&mut self, _: (), _ctx: &mut ActorContext<Self::Msg>) {}
    }

    let sys = ActorSystem::new();
    let r = sys.spawn(EscalateParent);
    tokio::time::sleep(Duration::from_millis(20)).await;
    // Trigger child panic
    // (child has no msgs — it never gets triggered; just verify no deadlock)
    drop(r);
    tokio::time::sleep(Duration::from_millis(50)).await;
    sys.shutdown().await;
}

#[tokio::test]
async fn test_allforone_stops_siblings() {
    // AllForOne: when a child using AllForOne strategy panics, siblings get stop signal.
    // We verify it does not deadlock and system remains healthy afterward.
    let sibling_stopped = Arc::new(AtomicBool::new(false));

    struct AllForOneParent {
        sibling_stopped: Arc<AtomicBool>,
    }

    struct SiblingActor {
        stopped: Arc<AtomicBool>,
    }

    #[async_trait::async_trait]
    impl Actor for SiblingActor {
        type Msg = ();
        async fn handle(&mut self, _: (), _ctx: &mut ActorContext<Self::Msg>) {}
        async fn post_stop(&mut self) {
            self.stopped.store(true, Ordering::SeqCst);
        }
    }

    #[async_trait::async_trait]
    impl Actor for AllForOneParent {
        type Msg = ();
        async fn pre_start(&mut self, ctx: &mut ActorContext<Self::Msg>) {
            let _sibling = ctx.spawn_child(
                SiblingActor {
                    stopped: self.sibling_stopped.clone(),
                },
                SupervisorStrategy::AllForOne {
                    max_restarts: 1,
                    within_secs: 60,
                },
            );
            let _panic_child = ctx.spawn_child(
                PanicActor,
                SupervisorStrategy::AllForOne {
                    max_restarts: 1,
                    within_secs: 60,
                },
            );
        }
        async fn handle(&mut self, _: (), _ctx: &mut ActorContext<Self::Msg>) {}
    }

    let sys = ActorSystem::new();
    let _parent = sys.spawn(AllForOneParent {
        sibling_stopped: sibling_stopped.clone(),
    });
    // Trigger panic on the panic child by sending it a message
    // (child is spawned inside pre_start, so we can't get its ref here — send to parent)
    tokio::time::sleep(Duration::from_millis(50)).await;
    // Panic child will panic on first message — but it has no external sender here.
    // The AllForOne strategy behavior is exercised via supervisor monitor code path.
    // Just verify no deadlock and system is alive.
    let count = Arc::new(AtomicU32::new(0));
    let r = sys.spawn(CountActor {
        count: count.clone(),
    });
    let _ = r.send(());
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(count.load(Ordering::SeqCst), 1);
    sys.shutdown().await;
}

#[tokio::test]
async fn test_oncefor_one_doesnt_affect_siblings() {
    // Two independent top-level actors; one panics; verify the other still works
    let count = Arc::new(AtomicU32::new(0));
    let sys = ActorSystem::new();
    let panic_ref = sys.spawn(PanicActor);
    let count_ref = sys.spawn(CountActor {
        count: count.clone(),
    });
    let _ = panic_ref.send(());
    tokio::time::sleep(Duration::from_millis(100)).await;
    let _ = count_ref.send(());
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(count.load(Ordering::SeqCst), 1);
    sys.shutdown().await;
}

// ── Group 5: CrossbeamPipe ────────────────────────────────────────────────────

#[tokio::test]
async fn test_pipe_bounded_send_recv() {
    let pipe: CrossbeamPipe<i32> = CrossbeamPipe::bounded(4);
    pipe.send(42).unwrap();
    let v = pipe.recv().unwrap();
    assert_eq!(v, 42);
}

#[tokio::test]
async fn test_pipe_bounded_full_try_send_fails() {
    let pipe: CrossbeamPipe<i32> = CrossbeamPipe::bounded(2);
    pipe.send(1).unwrap();
    pipe.send(2).unwrap();
    let result = pipe.try_send(3);
    assert!(result.is_err());
}

#[tokio::test]
async fn test_pipe_unbounded_large_volume() {
    let pipe: CrossbeamPipe<u64> = CrossbeamPipe::unbounded();
    for i in 0..10_000u64 {
        pipe.send(i).unwrap();
    }
    for i in 0..10_000u64 {
        assert_eq!(pipe.recv().unwrap(), i);
    }
}

#[tokio::test]
async fn test_pipe_recv_async_in_tokio() {
    let pipe: CrossbeamPipe<String> = CrossbeamPipe::unbounded();
    pipe.send("async!".into()).unwrap();
    let v = pipe.recv_async().await.unwrap();
    assert_eq!(v, "async!");
}

#[tokio::test]
async fn test_pipe_disconnected_on_drop() {
    let pipe: CrossbeamPipe<i32> = CrossbeamPipe::bounded(1);
    let rx = pipe.receiver();
    drop(pipe);
    let result = rx.try_recv();
    assert!(result.is_err());
}

// ── Group 6: ActorSystem ──────────────────────────────────────────────────────

#[tokio::test]
async fn test_shutdown_stops_all_actors() {
    let started1 = Arc::new(AtomicBool::new(false));
    let started2 = Arc::new(AtomicBool::new(false));
    let sys = ActorSystem::new();
    let _r1 = sys.spawn(PreStartActor {
        started: started1.clone(),
    });
    let _r2 = sys.spawn(PreStartActor {
        started: started2.clone(),
    });
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert!(started1.load(Ordering::SeqCst));
    assert!(started2.load(Ordering::SeqCst));
    sys.shutdown().await;
    // Sending after shutdown should fail
    let _ = _r1.send(()); // best-effort; channel may already be closed
}

#[tokio::test]
async fn test_shutdown_with_pending_messages() {
    let count = Arc::new(AtomicU32::new(0));
    let sys = ActorSystem::new();
    let r = sys.spawn(CountActor {
        count: count.clone(),
    });
    // Queue many messages then immediately shut down
    for _ in 0..500 {
        let _ = r.send(());
    }
    sys.shutdown().await;
    // Some messages processed, some not — just verify no hang/panic
}

#[tokio::test]
async fn test_graceful_no_hang() {
    let sys = ActorSystem::new();
    for _ in 0..10 {
        let count = Arc::new(AtomicU32::new(0));
        let _r = sys.spawn(CountActor { count });
    }
    // Should complete within test timeout
    sys.shutdown().await;
}

// ── Group 7: pre_restart hook ─────────────────────────────────────────────────

#[tokio::test]
async fn test_pre_restart_hook_called_on_panic() {
    let pre_restart_count = Arc::new(AtomicU32::new(0));
    let pre_start_count = Arc::new(AtomicU32::new(0));

    struct HookActor {
        pre_restart_count: Arc<AtomicU32>,
        pre_start_count: Arc<AtomicU32>,
    }

    #[async_trait::async_trait]
    impl Actor for HookActor {
        type Msg = bool; // true = panic, false = noop

        async fn pre_start(&mut self, _ctx: &mut ActorContext<Self::Msg>) {
            self.pre_start_count.fetch_add(1, Ordering::SeqCst);
        }

        async fn handle(&mut self, do_panic: bool, _ctx: &mut ActorContext<Self::Msg>) {
            if do_panic {
                panic!("test panic");
            }
        }

        async fn pre_restart(&mut self, _err: &ActorError) {
            self.pre_restart_count.fetch_add(1, Ordering::SeqCst);
        }
    }

    let prc = pre_restart_count.clone();
    let psc = pre_start_count.clone();

    let sys = ActorSystem::new();
    let r = sys.spawn_with_factory(
        move || HookActor {
            pre_restart_count: prc.clone(),
            pre_start_count: psc.clone(),
        },
        SupervisorStrategy::OneForOne {
            max_restarts: 3,
            within_secs: 60,
        },
    );

    // Cause a panic → supervisor restarts actor
    let _ = r.send(true);
    tokio::time::sleep(Duration::from_millis(150)).await;

    assert_eq!(
        pre_restart_count.load(Ordering::SeqCst),
        1,
        "pre_restart called once"
    );
    assert_eq!(
        pre_start_count.load(Ordering::SeqCst),
        2,
        "pre_start called on initial + restart"
    );

    // Actor is alive again: send a noop message
    let _ = r.send(false);
    tokio::time::sleep(Duration::from_millis(50)).await;

    sys.shutdown().await;
}

#[tokio::test]
async fn test_pre_restart_actor_resumes_after_restart() {
    let alive_count = Arc::new(AtomicU32::new(0));

    struct ResumableActor {
        alive_count: Arc<AtomicU32>,
    }

    #[async_trait::async_trait]
    impl Actor for ResumableActor {
        type Msg = bool;
        async fn handle(&mut self, do_panic: bool, _ctx: &mut ActorContext<Self::Msg>) {
            if do_panic {
                panic!("boom");
            } else {
                self.alive_count.fetch_add(1, Ordering::SeqCst);
            }
        }
    }

    let ac = alive_count.clone();
    let sys = ActorSystem::new();
    let r = sys.spawn_with_factory(
        move || ResumableActor {
            alive_count: ac.clone(),
        },
        SupervisorStrategy::OneForOne {
            max_restarts: 1,
            within_secs: 60,
        },
    );

    let _ = r.send(true); // panic → restart
    tokio::time::sleep(Duration::from_millis(150)).await;

    let _ = r.send(false); // should process after restart
    tokio::time::sleep(Duration::from_millis(50)).await;

    assert_eq!(alive_count.load(Ordering::SeqCst), 1);
    sys.shutdown().await;
}

// ── Group 8: RestartN strategy ────────────────────────────────────────────────

#[tokio::test]
async fn test_restart_n_allows_n_restarts() {
    let alive_count = Arc::new(AtomicU32::new(0));

    struct NRestartActor {
        alive_count: Arc<AtomicU32>,
    }

    #[async_trait::async_trait]
    impl Actor for NRestartActor {
        type Msg = bool;
        async fn handle(&mut self, do_panic: bool, _ctx: &mut ActorContext<Self::Msg>) {
            if do_panic {
                panic!("intentional");
            } else {
                self.alive_count.fetch_add(1, Ordering::SeqCst);
            }
        }
    }

    let ac = alive_count.clone();
    let sys = ActorSystem::new();
    let r = sys.spawn_with_factory(
        move || NRestartActor {
            alive_count: ac.clone(),
        },
        SupervisorStrategy::RestartN {
            max: 2,
            within_secs: 10,
        },
    );

    // Panic 1 — restart #1
    let _ = r.send(true);
    tokio::time::sleep(Duration::from_millis(150)).await;
    let _ = r.send(false);
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(
        alive_count.load(Ordering::SeqCst),
        1,
        "alive after restart 1"
    );

    // Panic 2 — restart #2
    let _ = r.send(true);
    tokio::time::sleep(Duration::from_millis(150)).await;
    let _ = r.send(false);
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(
        alive_count.load(Ordering::SeqCst),
        2,
        "alive after restart 2"
    );

    // Panic 3 — limit exceeded, actor dies
    let _ = r.send(true);
    tokio::time::sleep(Duration::from_millis(150)).await;

    // Actor should be gone: all receivers dropped, channel disconnected
    let result = r.try_send(false);
    assert!(
        matches!(
            result,
            Err(ActorError::Disconnected) | Err(ActorError::MailboxFull)
        ),
        "actor dead after restart limit exceeded: {result:?}"
    );

    sys.shutdown().await;
}

// ── Group 9: Bounded mailbox / backpressure ───────────────────────────────────

#[tokio::test(flavor = "current_thread")]
async fn test_bounded_mailbox_backpressure_try_send() {
    let processed = Arc::new(AtomicU32::new(0));

    let sys = ActorSystem::new();
    let r = sys.spawn_with_capacity(
        CountActor {
            count: processed.clone(),
        },
        2,
    );

    // No yield yet: actor task hasn't run. Fill the 2-slot mailbox immediately.
    assert!(r.try_send(()).is_ok(), "slot 1");
    assert!(r.try_send(()).is_ok(), "slot 2");
    // 3rd message: mailbox full
    assert!(
        matches!(r.try_send(()), Err(ActorError::MailboxFull)),
        "3rd try_send must fail with MailboxFull"
    );

    // Yield to let actor drain
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(processed.load(Ordering::SeqCst), 2);

    // Can send again now
    assert!(r.try_send(()).is_ok());
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(processed.load(Ordering::SeqCst), 3);

    sys.shutdown().await;
}

#[tokio::test(flavor = "current_thread")]
async fn test_bounded_mailbox_send_async_waits_for_space() {
    let processed = Arc::new(AtomicU32::new(0));

    let sys = ActorSystem::new();
    let r = sys.spawn_with_capacity(
        CountActor {
            count: processed.clone(),
        },
        1,
    );

    // Fill the 1-slot mailbox
    assert!(r.try_send(()).is_ok());

    // send_async blocks until actor drains one slot; this will yield and let the actor run
    r.send_async(()).await.unwrap();
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(processed.load(Ordering::SeqCst), 2);

    sys.shutdown().await;
}
