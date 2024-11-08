//! Implementation of  [`ProcessControlBlock`]

use super::id::RecycleAllocator;
use super::manager::insert_into_pid2process;
use super::TaskControlBlock;
use super::{add_task, SignalFlags};
use super::{pid_alloc, PidHandle};
use crate::fs::{File, Stdin, Stdout};
use crate::mm::{translated_refmut, MemorySet, KERNEL_SPACE};
use crate::sync::{Condvar, Mutex, Semaphore, UPSafeCell};
use crate::trap::{trap_handler, TrapContext};
use alloc::string::String;
use alloc::sync::{Arc, Weak};
use alloc::vec;
use alloc::vec::Vec;
use core::cell::RefMut;

/// Process Control Block
pub struct ProcessControlBlock {
    /// immutable
    pub pid: PidHandle,
    /// mutable
    inner: UPSafeCell<ProcessControlBlockInner>,
}

/// Inner of Process Control Block
pub struct ProcessControlBlockInner {
    /// is zombie?
    pub is_zombie: bool,
    /// memory set(address space)
    pub memory_set: MemorySet,
    /// parent process
    pub parent: Option<Weak<ProcessControlBlock>>,
    /// children process
    pub children: Vec<Arc<ProcessControlBlock>>,
    /// exit code
    pub exit_code: i32,
    /// file descriptor table
    pub fd_table: Vec<Option<Arc<dyn File + Send + Sync>>>,
    /// signal flags
    pub signals: SignalFlags,
    /// tasks(also known as threads)
    pub tasks: Vec<Option<Arc<TaskControlBlock>>>,
    /// task resource allocator
    pub task_res_allocator: RecycleAllocator,
    /// mutex list
    pub mutex_list: Vec<Option<Arc<dyn Mutex>>>,
    /// semaphore list
    pub semaphore_list: Vec<Option<Arc<Semaphore>>>,
    /// condvar list
    pub condvar_list: Vec<Option<Arc<Condvar>>>,

    /// Available for mutex
    pub mutex_avail: Vec<usize>,
    /// Available for semaphore
    pub semaphore_avail: Vec<usize>,

    /// Allocation for mutex
    pub mutex_alloc: Vec<Vec<usize>>,
    /// Allocation for semaphore
    pub semaphore_alloc: Vec<Vec<usize>>,

    ///Need for mutex
    pub mutex_need: Vec<Vec<usize>>,
    ///Need for semaphore
    pub semaphore_need: Vec<Vec<usize>>,

    ///死锁检测是否开启
    pub deadlock_detect: bool,
    ///开始时间
    pub start_time: usize,
}

impl ProcessControlBlockInner {
    #[allow(unused)]
    /// get the address of app's page table
    pub fn get_user_token(&self) -> usize {
        self.memory_set.token()
    }
    /// allocate a new file descriptor
    pub fn alloc_fd(&mut self) -> usize {
        if let Some(fd) = (0..self.fd_table.len()).find(|fd| self.fd_table[*fd].is_none()) {
            fd
        } else {
            self.fd_table.push(None);
            self.fd_table.len() - 1
        }
    }
    /// allocate a new task id
    pub fn alloc_tid(&mut self) -> usize {
        self.task_res_allocator.alloc()
    }
    /// deallocate a task id
    pub fn dealloc_tid(&mut self, tid: usize) {
        self.task_res_allocator.dealloc(tid)
    }
    /// the count of tasks(threads) in this process
    pub fn thread_count(&self) -> usize {
        self.tasks.len()
    }
    /// get a task with tid in this process
    pub fn get_task(&self, tid: usize) -> Arc<TaskControlBlock> {
        self.tasks[tid].as_ref().unwrap().clone()
    }
    ///创建线程时同步修改线程下的avail alloc need向量
    pub fn pro_add_task(&mut self, _tid: usize) {
        let len_mutex = self.mutex_avail.len();
        let len_semaphore = self.semaphore_avail.len();
        if _tid == 0 {
            self.mutex_alloc.push(vec![0;len_mutex]);
            self.mutex_need.push(vec![0;len_mutex]);
            self.semaphore_alloc.push(vec![0;len_semaphore]);
            self.semaphore_need.push(vec![0;len_semaphore]);
            return;
        }

        if _tid < self.mutex_alloc.len(){
            self.mutex_alloc[_tid] = vec![0;len_mutex];
            self.mutex_need[_tid]= vec![0;len_mutex];
            self.semaphore_alloc[_tid] = vec![0;len_semaphore];
            self.semaphore_need[_tid]= vec![0;len_semaphore];
        }else {
            while self.mutex_alloc.len() <= _tid {
                self.mutex_alloc.push(vec![0; len_mutex]);
                self.mutex_need.push(vec![0; len_mutex]);
            }
            while self.semaphore_alloc.len() <= _tid {
                self.semaphore_alloc.push(vec![0; len_semaphore]);
                self.semaphore_need.push(vec![0; len_semaphore]);
            }
        }
    }
    ///死锁检测
    pub fn deadlock_detect_mutex(&self) -> isize {
        let mut work = self.mutex_avail.clone();
        let mut finish = vec![false; self.mutex_alloc.len()];
        let mut flag = true;
        while flag {
            flag = false;
            for i in 0..self.mutex_alloc.len() {
                if !finish[i] {
                    let mut j = 0;
                    while j < self.mutex_avail.len() {
                        if self.mutex_need[i][j] > work[j] {
                            break;
                        }
                        j += 1;
                    }
                    if j == self.mutex_avail.len() {
                        finish[i] = true;
                        flag = true;
                        for j in 0..self.mutex_avail.len() {
                            work[j] += self.mutex_alloc[i][j];
                        }
                    }
                }
            }
        }
        if finish.iter().all(|&x| x) {
            0
        } else {
            -1
        }
    }
    pub fn deadlock_detect_semaphore(&self) -> isize {
        let mut work = self.semaphore_avail.clone();
        let mut finish = vec![false; self.semaphore_alloc.len()];
        let mut flag = true;
        while flag {
            flag = false;
            for i in 0..self.semaphore_alloc.len() {
                if !finish[i] {
                    let mut j = 0;
                    while j < self.semaphore_avail.len() {
                        if self.semaphore_need[i][j] > work[j] {
                            break;
                        }
                        j += 1;
                    }
                    if j == self.semaphore_avail.len() {
                        finish[i] = true;
                        flag = true;
                        for j in 0..self.semaphore_avail.len() {
                            work[j] += self.semaphore_alloc[i][j];
                        }
                    }
                }
            }
        }
        if finish.iter().all(|&x| x) {
            0
        } else {
            -1
        }
    }

    pub fn deadlock_detect(&self) -> isize {
        if self.deadlock_detect {
            if self.deadlock_detect_mutex() == -1 || self.deadlock_detect_semaphore() == -1 {
                -1
            } else {
                0
            }
        } else {
            0
        }
    }
}

impl ProcessControlBlock {
    /// inner_exclusive_access
    pub fn inner_exclusive_access(&self) -> RefMut<'_, ProcessControlBlockInner> {
        self.inner.exclusive_access()
    }
    /// new process from elf file
    pub fn new(elf_data: &[u8]) -> Arc<Self> {
        trace!("kernel: ProcessControlBlock::new");
        // memory_set with elf program headers/trampoline/trap context/user stack
        let (memory_set, ustack_base, entry_point) = MemorySet::from_elf(elf_data);
        // allocate a pid
        let pid_handle = pid_alloc();
        let process = Arc::new(Self {
            pid: pid_handle,
            inner: unsafe {
                UPSafeCell::new(ProcessControlBlockInner {
                    is_zombie: false,
                    memory_set,
                    parent: None,
                    children: Vec::new(),
                    exit_code: 0,
                    fd_table: vec![
                        // 0 -> stdin
                        Some(Arc::new(Stdin)),
                        // 1 -> stdout
                        Some(Arc::new(Stdout)),
                        // 2 -> stderr
                        Some(Arc::new(Stdout)),
                    ],
                    signals: SignalFlags::empty(),
                    tasks: Vec::new(),
                    task_res_allocator: RecycleAllocator::new(),
                    mutex_list: Vec::new(),
                    semaphore_list: Vec::new(),
                    condvar_list: Vec::new(),
                    mutex_avail: Vec::new(),
                    semaphore_avail: Vec::new(),
                    mutex_alloc: Vec::new(),
                    semaphore_alloc: Vec::new(),
                    mutex_need: Vec::new(),
                    semaphore_need: Vec::new(),
                    deadlock_detect: false,
                    start_time: 0,
                })
            },
        });
        // create a main thread, we should allocate ustack and trap_cx here
        let task = Arc::new(TaskControlBlock::new(
            Arc::clone(&process),
            ustack_base,
            true,
        ));
        // prepare trap_cx of main thread
        let task_inner = task.inner_exclusive_access();
        let trap_cx = task_inner.get_trap_cx();
        let ustack_top = task_inner.res.as_ref().unwrap().ustack_top();
        let kstack_top = task.kstack.get_top();
        drop(task_inner);
        *trap_cx = TrapContext::app_init_context(
            entry_point,
            ustack_top,
            KERNEL_SPACE.exclusive_access().token(),
            kstack_top,
            trap_handler as usize,
        );
        // add main thread to the process
        let mut process_inner = process.inner_exclusive_access();
        process_inner.tasks.push(Some(Arc::clone(&task)));
        process_inner.pro_add_task(0);
        drop(process_inner);
        insert_into_pid2process(process.getpid(), Arc::clone(&process));
        // add main thread to scheduler
        add_task(task);
        process
    }

    /// Only support processes with a single thread.
    pub fn exec(self: &Arc<Self>, elf_data: &[u8], args: Vec<String>) {
        trace!("kernel: exec");
        assert_eq!(self.inner_exclusive_access().thread_count(), 1);
        // memory_set with elf program headers/trampoline/trap context/user stack
        trace!("kernel: exec .. MemorySet::from_elf");
        let (memory_set, ustack_base, entry_point) = MemorySet::from_elf(elf_data);
        let new_token = memory_set.token();
        // substitute memory_set
        trace!("kernel: exec .. substitute memory_set");
        self.inner_exclusive_access().memory_set = memory_set;
        // then we alloc user resource for main thread again
        // since memory_set has been changed
        trace!("kernel: exec .. alloc user resource for main thread again");
        let task = self.inner_exclusive_access().get_task(0);
        let mut task_inner = task.inner_exclusive_access();
        task_inner.res.as_mut().unwrap().ustack_base = ustack_base;
        task_inner.res.as_mut().unwrap().alloc_user_res();
        task_inner.trap_cx_ppn = task_inner.res.as_mut().unwrap().trap_cx_ppn();
        // push arguments on user stack
        trace!("kernel: exec .. push arguments on user stack");
        let mut user_sp = task_inner.res.as_mut().unwrap().ustack_top();
        user_sp -= (args.len() + 1) * core::mem::size_of::<usize>();
        let argv_base = user_sp;
        let mut argv: Vec<_> = (0..=args.len())
            .map(|arg| {
                translated_refmut(
                    new_token,
                    (argv_base + arg * core::mem::size_of::<usize>()) as *mut usize,
                )
            })
            .collect();
        *argv[args.len()] = 0;
        for i in 0..args.len() {
            user_sp -= args[i].len() + 1;
            *argv[i] = user_sp;
            let mut p = user_sp;
            for c in args[i].as_bytes() {
                *translated_refmut(new_token, p as *mut u8) = *c;
                p += 1;
            }
            *translated_refmut(new_token, p as *mut u8) = 0;
        }
        // make the user_sp aligned to 8B for k210 platform
        user_sp -= user_sp % core::mem::size_of::<usize>();
        // initialize trap_cx
        trace!("kernel: exec .. initialize trap_cx");
        let mut trap_cx = TrapContext::app_init_context(
            entry_point,
            user_sp,
            KERNEL_SPACE.exclusive_access().token(),
            task.kstack.get_top(),
            trap_handler as usize,
        );
        trap_cx.x[10] = args.len();
        trap_cx.x[11] = argv_base;
        *task_inner.get_trap_cx() = trap_cx;
    }

    /// Only support processes with a single thread.
    pub fn fork(self: &Arc<Self>) -> Arc<Self> {
        trace!("kernel: fork");
        let mut parent = self.inner_exclusive_access();
        assert_eq!(parent.thread_count(), 1);
        // clone parent's memory_set completely including trampoline/ustacks/trap_cxs
        let memory_set = MemorySet::from_existed_user(&parent.memory_set);
        // alloc a pid
        let pid = pid_alloc();
        // copy fd table
        let mut new_fd_table: Vec<Option<Arc<dyn File + Send + Sync>>> = Vec::new();
        for fd in parent.fd_table.iter() {
            if let Some(file) = fd {
                new_fd_table.push(Some(file.clone()));
            } else {
                new_fd_table.push(None);
            }
        }
        // create child process pcb
        let child = Arc::new(Self {
            pid,
            inner: unsafe {
                UPSafeCell::new(ProcessControlBlockInner {
                    is_zombie: false,
                    memory_set,
                    parent: Some(Arc::downgrade(self)),
                    children: Vec::new(),
                    exit_code: 0,
                    fd_table: new_fd_table,
                    signals: SignalFlags::empty(),
                    tasks: Vec::new(),
                    task_res_allocator: RecycleAllocator::new(),
                    mutex_list: Vec::new(),
                    semaphore_list: Vec::new(),
                    condvar_list: Vec::new(),
                    mutex_avail: parent.mutex_avail.clone(),
                    semaphore_avail: parent.semaphore_avail.clone(),
                    mutex_alloc: parent.mutex_alloc.clone(),
                    semaphore_alloc: parent.semaphore_alloc.clone(),
                    mutex_need: parent.mutex_need.clone(),
                    semaphore_need: parent.semaphore_need.clone(),
                    deadlock_detect: false,
                    start_time: 0,
                })
            },
        });
        // add child
        parent.children.push(Arc::clone(&child));
        // create main thread of child process
        let task = Arc::new(TaskControlBlock::new(
            Arc::clone(&child),
            parent
                .get_task(0)
                .inner_exclusive_access()
                .res
                .as_ref()
                .unwrap()
                .ustack_base(),
            // here we do not allocate trap_cx or ustack again
            // but mention that we allocate a new kstack here
            false,
        ));
        // attach task to child process
        let mut child_inner = child.inner_exclusive_access();
        child_inner.tasks.push(Some(Arc::clone(&task)));
        child_inner.pro_add_task(0);
        drop(child_inner);
        // modify kstack_top in trap_cx of this thread
        let task_inner = task.inner_exclusive_access();
        let trap_cx = task_inner.get_trap_cx();
        trap_cx.kernel_sp = task.kstack.get_top();
        drop(task_inner);
        insert_into_pid2process(child.getpid(), Arc::clone(&child));
        // add this thread to scheduler
        add_task(task);
        child
    }
    /// get pid
    pub fn getpid(&self) -> usize {
        self.pid.0
    }

    
}
