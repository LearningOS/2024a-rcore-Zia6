//!Implementation of [`TaskManager`]
use super::TaskControlBlock;
use crate::sync::UPSafeCell;
use alloc::sync::Arc;
use lazy_static::*;
use alloc::vec::Vec;
///A array of `TaskControlBlock` that is thread-safe
pub struct TaskManager {
    ready_queue: Vec<Arc<TaskControlBlock>>,
}

/// A simple FIFO scheduler.
impl TaskManager {
    ///Creat an empty TaskManager
    pub fn new() -> Self {
        Self {
            ready_queue: Vec::new(),
        }
    }
    /// Add process back to ready queue
    pub fn add(&mut self, task: Arc<TaskControlBlock>) {
        //实现基于stride的小顶堆
        self.ready_queue.push(task);
        let mut i = self.ready_queue.len() - 1;
        while i > 0 {
            let p = (i - 1) / 2;
            if self.ready_queue[p].inner_exclusive_access().stride
                > self.ready_queue[i].inner_exclusive_access().stride
            {
                self.ready_queue.swap(p, i);
                i = p;
            } else {
                break;
            }
        }
    }
     /// Take a process out of the ready queue
     pub fn fetch(&mut self) -> Option<Arc<TaskControlBlock>> {
        if self.ready_queue.is_empty() {
            return None;
        }
        let task = self.ready_queue[0].clone();
        // 使用pop移除堆顶元素
        if let Some(last) = self.ready_queue.pop() {
            if !self.ready_queue.is_empty() {
                self.ready_queue[0] = last;
                let mut i = 0;
                while i * 2 + 1 < self.ready_queue.len() {
                    let mut j = i * 2 + 1;
                    if j + 1 < self.ready_queue.len()
                        && self.ready_queue[j].inner_exclusive_access().stride
                            > self.ready_queue[j + 1].inner_exclusive_access().stride
                    {
                        j += 1;
                    }
                    if self.ready_queue[i].inner_exclusive_access().stride
                        > self.ready_queue[j].inner_exclusive_access().stride
                    {
                        self.ready_queue.swap(i, j);
                        i = j;
                    } else {
                        break;
                    }
                }
            }
        }
        Some(task)
    }
}

lazy_static! {
    /// TASK_MANAGER instance through lazy_static!
    pub static ref TASK_MANAGER: UPSafeCell<TaskManager> =
        unsafe { UPSafeCell::new(TaskManager::new()) };
}

/// Add process to ready queue
pub fn add_task(task: Arc<TaskControlBlock>) {
    //trace!("kernel: TaskManager::add_task");
    TASK_MANAGER.exclusive_access().add(task);
}

/// Take a process out of the ready queue
pub fn fetch_task() -> Option<Arc<TaskControlBlock>> {
    //trace!("kernel: TaskManager::fetch_task");
    TASK_MANAGER.exclusive_access().fetch()
}
