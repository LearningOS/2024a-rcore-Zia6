//! Process management syscalls
use crate::mm::translated_byte_buffer;
use crate::{
    config::MAX_SYSCALL_NUM,
    task::{
        change_program_brk, current_sys_call_time, current_user_token, exit_current_and_run_next,
        get_start_time, suspend_current_and_run_next, TaskStatus,insert_framed_area,remove_area
    },
    timer::{get_time_ms, get_time_us},
};

#[repr(C)]
#[derive(Debug)]
pub struct TimeVal {
    pub sec: usize,
    pub usec: usize,
}

/// Task information
#[allow(dead_code)]
pub struct TaskInfo {
    /// Task status in it's life cycle
    status: TaskStatus,
    /// The numbers of syscall called by task
    syscall_times: [u32; MAX_SYSCALL_NUM],
    /// Total running time of task
    time: usize,
}
/// task exits and submit an exit code
pub fn sys_exit(_exit_code: i32) -> ! {
    trace!("kernel: sys_exit");
    exit_current_and_run_next();
    panic!("Unreachable in sys_exit!");
}

/// current task gives up resources for other tasks
pub fn sys_yield() -> isize {
    trace!("kernel: sys_yield");
    suspend_current_and_run_next();
    0
}
/// copy_out
pub fn copy_out(dst: *mut u8, src: *const u8, len: usize) {
    let token = current_user_token();
    let buffers = translated_byte_buffer(token, dst, len);
    let mut offset = 0;
    for buffer in buffers {
        unsafe {
            buffer.copy_from_slice(core::slice::from_raw_parts(src.add(offset), buffer.len()));
        }
        offset += buffer.len();
    }
}

/// YOUR JOB: get time with second and microsecond
/// HINT: You might reimplement it with virtual memory management.
/// HINT: What if [`TimeVal`] is splitted by two pages ?
pub fn sys_get_time(ts: *mut TimeVal, _tz: usize) -> isize {
    trace!("kernel: sys_get_time");

    let now = get_time_us();
    let time_val = TimeVal {
        sec: now / 1_000_000,
        usec: now % 1_000_000,
    };
    copy_out(
        ts as *mut u8,
        &time_val as *const TimeVal as *const u8,
        core::mem::size_of::<TimeVal>(),
    );
    0
}

/// YOUR JOB: Finish sys_task_info to pass testcases
/// HINT: You might reimplement it with virtual memory management.
/// HINT: What if [`TaskInfo`] is splitted by two pages ?
pub fn sys_task_info(_ti: *mut TaskInfo) -> isize {
    trace!("kernel: sys_task_info");
    let now = get_time_ms();
    let task_info = TaskInfo {
        status: TaskStatus::Running,
        syscall_times: current_sys_call_time(),
        time: now - get_start_time(),
    };
    copy_out(
        _ti as *mut u8,
        &task_info as *const TaskInfo as *const u8,
        core::mem::size_of::<TaskInfo>(),
    );
    0
}

// YOUR JOB: Implement mmap.
pub fn sys_mmap(_start: usize, _len: usize, _port: usize) -> isize {
    trace!("kernel: sys_mmap NOT IMPLEMENTED YET!");
    if (_start % 4096 != 0) || (_port & (!0x7)) != 0 || (_port & 0x7) == 0 {
        return -1;
    }
    let _end = _start + _len;
    // print!("start:{},len:{},port:{}",_start,_len,_port);
    if let Err(msg) = insert_framed_area(_start, _end, _port) {
        error!("{}", msg);
        return -1;
    }
    0
}

// YOUR JOB: Implement munmap.
pub fn sys_munmap(_start: usize, _len: usize) -> isize {
    trace!("kernel: sys_munmap NOT IMPLEMENTED YET!");
    let _end = _start + _len;
    if _start % 4096 != 0 {
        return -1;
    }
    if let Err(msg) = remove_area(_start, _end) {
        error!("{}", msg);
        return -1;
    }
    0
}
/// change data segment size
pub fn sys_sbrk(size: i32) -> isize {
    trace!("kernel: sys_sbrk");
    if let Some(old_brk) = change_program_brk(size) {
        old_brk as isize
    } else {
        -1
    }
}

// ///copy_out
// pub fn copy_out(dst: *mut u8, src: *const u8, len: usize) {
//     let buffers = translated_byte_buffer(current_user_token(), src, len);
//     let mut offset = 0;
//     for buffer in buffers {
//         unsafe {
//             core::ptr::copy_nonoverlapping(buffer.as_ptr(), dst.add(offset), buffer.len());
//             //调试是否copy成功
//             print!("{:?}",core::str::from_utf8_unchecked(core::slice::from_raw_parts(dst.add(offset),buffer.len())));
//         }
//         offset += buffer.len();
//     }
// }
