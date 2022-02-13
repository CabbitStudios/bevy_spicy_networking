mod bevy_runtime;

use std::future::Future;

/// A runtime abstraction allowing you to use any runtime for spicy
pub trait Runtime: 'static + Send + Sync{

    /// Associated handle
    type JoinHandle: JoinHandle;

    /// Create a long running background task.
    fn spawn(&self, task: impl Future<Output = ()> + Send + 'static) -> Self::JoinHandle;
}

/// A runtime abstraction allowing you to use any runtime with spicy
pub trait JoinHandle: 'static + Send + Sync{

    /// Stop the task.
    fn abort(&mut self);
}