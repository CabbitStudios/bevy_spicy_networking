use crate::Runtime;

use super::JoinHandle;

impl Runtime for bevy::tasks::TaskPool{
    type JoinHandle = Option<bevy::tasks::Task<()>>;

    fn spawn(&self, task: impl std::future::Future<Output = ()> + Send  + 'static) -> Self::JoinHandle {
        Some(self.spawn(task))
    }
}

impl JoinHandle for Option<bevy::tasks::Task<()>>{
    fn abort(&mut self) {
        self.take().unwrap().cancel();
    }
}