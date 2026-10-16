//! Resource guard — Perihel must never compete with the user's real work.
//!
//! Two mechanisms:
//! 1. **Process priority** — the whole process runs at `BelowNormal` priority, so the
//!    OS scheduler always favors foreground apps under CPU contention.
//! 2. **Gate checks** — before any heavy step (loading a model, starting LLM inference)
//!    the guard checks free RAM and recent CPU load. When the machine is busy the heavy
//!    work is skipped/deferred, never queued up to thrash the system.

use std::time::Duration;

#[cfg(windows)]
use windows_sys::Win32::System::SystemInformation::{GetSystemInfo, SYSTEM_INFO};
#[cfg(windows)]
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, SetPriorityClass, BELOW_NORMAL_PRIORITY_CLASS,
};

/// Drop the process priority so foreground apps always win under contention.
#[cfg(windows)]
pub fn set_priority_idle() {
    unsafe {
        let h = GetCurrentProcess();
        // BELOW_NORMAL keeps background indexing responsive without starving the GUI;
        // Perihel is a companion widget, never a foreground workload.
        SetPriorityClass(h, BELOW_NORMAL_PRIORITY_CLASS);
    }
}

#[cfg(not(windows))]
pub fn set_priority_idle() {
    let _ = std::process::Command::new("renice")
        .args(["10", &std::process::id().to_string()])
        .status();
}

/// Number of logical processors (for CPU load sampling).
#[cfg(windows)]
fn processor_count() -> usize {
    unsafe {
        let mut info: SYSTEM_INFO = std::mem::zeroed();
        GetSystemInfo(&mut info);
        info.dwNumberOfProcessors as usize
    }
}

#[cfg(not(windows))]
fn processor_count() -> usize {
    std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4)
}

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Free physical RAM in MB (via PowerShell — dependency-free, cached by the OS).
pub fn ram_free_mb() -> u32 {
    #[cfg(windows)]
    {
        let mut cmd = std::process::Command::new("powershell");
        cmd.creation_flags(CREATE_NO_WINDOW);
        let out = cmd
            .args(["-NoProfile", "-Command", "(Get-CimInstance Win32_OperatingSystem).FreePhysicalMemory / 1024"])
            .creation_flags(0x0800_0000)
            .output();
        if let Ok(o) = out {
            if let Ok(v) = String::from_utf8_lossy(&o.stdout).trim().parse::<u32>() {
                return v;
            }
        }
        u32::MAX // unknown → never gate on it
    }
    #[cfg(not(windows))]
    {
        if let Ok(s) = std::fs::read_to_string("/proc/meminfo") {
            for line in s.lines() {
                if let Some(rest) = line.strip_prefix("MemAvailable:") {
                    if let Ok(kb) = rest.trim().trim_end_matches(" kB").parse::<u32>() {
                        return kb / 1024;
                    }
                }
            }
        }
        u32::MAX
    }
}

/// Rough CPU load percentage over a short sample window (GetSystemTimes based).
#[cfg(windows)]
pub fn cpu_load_percent(sample: Duration) -> u8 {
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::FILETIME;
        use windows_sys::Win32::System::Threading::GetSystemTimes;
        fn cpu_times() -> Option<(u64, u64)> {
            unsafe {
                let mut idle = FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 };
                let mut kern = FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 };
                let mut user = FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 };
                if GetSystemTimes(&mut idle, &mut kern, &mut user) == 0 {
                    return None;
                }
                fn ft(f: FILETIME) -> u64 {
                    ((f.dwHighDateTime as u64) << 32) | f.dwLowDateTime as u64
                }
                Some((ft(idle), ft(kern) + ft(user)))
            }
        }
        let a = cpu_times();
        std::thread::sleep(sample);
        let b = cpu_times();
        if let (Some((idle_a, total_a)), Some((idle_b, total_b))) = (a, b) {
            let total = total_b.saturating_sub(total_a);
            let idle = idle_b.saturating_sub(idle_a);
            if total == 0 {
                return 0;
            }
            let pct = 100 - (idle * 100 / total);
            return pct.min(100) as u8;
        }
        0
    }
    #[cfg(not(windows))]
    {
        let _ = sample;
        0
    }
}

/// Gates heavy work behind current machine pressure.
pub struct ResourceGuard {
    pub enabled: bool,
    pub min_free_ram_mb: u32,
    pub max_cpu_load_pct: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pressure {
    Ok,
    Busy,
}

impl ResourceGuard {
    pub fn conservative() -> Self {
        Self { enabled: true, min_free_ram_mb: 1200, max_cpu_load_pct: 85 }
    }

    /// Should heavy work (model load / inference) proceed right now?
    pub fn check(&self) -> Pressure {
        if !self.enabled {
            return Pressure::Ok;
        }
        if ram_free_mb() < self.min_free_ram_mb {
            return Pressure::Busy;
        }
        if cpu_load_percent(Duration::from_millis(120)) > self.max_cpu_load_pct {
            return Pressure::Busy;
        }
        Pressure::Ok
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guard_reports_something() {
        let g = ResourceGuard::conservative();
        let _ = g.check(); // just must not panic
        assert!(processor_count() >= 1);
    }
}
