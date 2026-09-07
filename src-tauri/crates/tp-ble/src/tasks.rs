//! Background work belongs to a connection and must stop when it is retired.

use tokio::task::JoinHandle;

#[derive(Default)]
pub(crate) struct ConnectionTasks(Vec<JoinHandle<()>>);

impl ConnectionTasks {
    pub(crate) fn push(&mut self, task: JoinHandle<()>) {
        self.0.push(task);
    }

    pub(crate) async fn shutdown(&mut self) {
        for task in &self.0 {
            task.abort();
        }
        for task in self.0.drain(..) {
            let _ = task.await;
        }
    }
}

impl Drop for ConnectionTasks {
    fn drop(&mut self) {
        for task in &self.0 {
            task.abort();
        }
    }
}
