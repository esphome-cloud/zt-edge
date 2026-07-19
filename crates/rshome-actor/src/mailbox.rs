pub(crate) struct Mailbox<M> {
    pub(crate) tx: flume::Sender<M>,
    pub(crate) rx: flume::Receiver<M>,
}

impl<M> Mailbox<M> {
    pub(crate) fn unbounded() -> Self {
        let (tx, rx) = flume::unbounded();
        Self { tx, rx }
    }

    pub(crate) fn bounded(cap: usize) -> Self {
        let (tx, rx) = flume::bounded(cap);
        Self { tx, rx }
    }
}
