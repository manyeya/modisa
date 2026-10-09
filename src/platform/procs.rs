// Facts about processes, asked of the kernel directly instead of spawning ps or lsof: a process's parent and command
// line, its children, its working directory. Agent detection asks every half second, for every pane, so this is the
// difference between a few system calls and a fork of ps that reads every process on the machine.
//
// macOS: libproc (proc_pidinfo, proc_listchildpids) and sysctl(KERN_PROCARGS2), the calls ps and lsof use themselves.
// Linux: /proc.

// A process: its parent and its command line, its arguments joined by spaces (what ps's `args` column shows).
#[derive(Clone, Debug, PartialEq)]
pub struct Info {
    pub pid: i32,
    pub ppid: i32,
    pub args: String,
}

#[cfg(test)]
pub fn alive(pid: i32) -> bool {
    pid > 0 && (unsafe { libc::kill(pid, 0) } == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM))
}

#[cfg(target_os = "macos")]
mod os {
    use super::Info;
    use std::mem::{size_of, zeroed};

    fn ppid(pid: i32) -> Option<i32> {
        let mut bsd: libc::proc_bsdinfo = unsafe { zeroed() };
        let size = size_of::<libc::proc_bsdinfo>() as i32;
        let n = unsafe { libc::proc_pidinfo(pid, libc::PROC_PIDTBSDINFO, 0, &mut bsd as *mut _ as *mut libc::c_void, size) };
        (n == size).then_some(bsd.pbi_ppid as i32)
    }

    // KERN_PROCARGS2: argc, the executable's path, padding, then argc NUL-terminated arguments (then the environment).
    fn args(pid: i32) -> Option<String> {
        let mut mib = [libc::CTL_KERN, libc::KERN_PROCARGS2, pid];
        let mut size: libc::size_t = 0;
        if unsafe { libc::sysctl(mib.as_mut_ptr(), 3, std::ptr::null_mut(), &mut size, std::ptr::null_mut(), 0) } != 0 || size < 4 {
            return None;
        }
        let mut buf = vec![0u8; size];
        if unsafe { libc::sysctl(mib.as_mut_ptr(), 3, buf.as_mut_ptr() as *mut libc::c_void, &mut size, std::ptr::null_mut(), 0) } != 0 {
            return None;
        }
        buf.truncate(size);
        parse_procargs(&buf)
    }

    pub fn parse_procargs(buf: &[u8]) -> Option<String> {
        let argc = i32::from_ne_bytes(buf.get(..4)?.try_into().ok()?) as usize;
        let mut rest = &buf[4..];
        let path_end = rest.iter().position(|&b| b == 0)?;
        rest = &rest[path_end..];
        let start = rest.iter().position(|&b| b != 0)?; // the padding after the path
        rest = &rest[start..];
        let args: Vec<String> = rest.split(|&b| b == 0).take(argc).map(|a| String::from_utf8_lossy(a).into_owned()).collect();
        Some(args.join(" "))
    }

    pub fn info(pid: i32) -> Option<Info> {
        Some(Info { pid, ppid: ppid(pid)?, args: args(pid).unwrap_or_default() })
    }

    pub fn children(pid: i32) -> Vec<i32> {
        let mut buf = vec![0i32; 64];
        loop {
            let bytes = (buf.len() * size_of::<i32>()) as i32;
            let n = unsafe { libc::proc_listchildpids(pid, buf.as_mut_ptr() as *mut libc::c_void, bytes) };
            if n < 0 {
                return vec![];
            }
            let n = n as usize;
            if n < buf.len() {
                buf.truncate(n);
                buf.retain(|&p| p > 0);
                return buf;
            }
            buf.resize(buf.len() * 4, 0); // full: there may be more
        }
    }

    pub fn cwd(pid: i32) -> Option<String> {
        let mut v: libc::proc_vnodepathinfo = unsafe { zeroed() };
        let size = size_of::<libc::proc_vnodepathinfo>() as i32;
        let n = unsafe { libc::proc_pidinfo(pid, libc::PROC_PIDVNODEPATHINFO, 0, &mut v as *mut _ as *mut libc::c_void, size) };
        if n != size {
            return None;
        }
        let path: Vec<u8> = v.pvi_cdir.vip_path.iter().flat_map(|row| row.iter()).map(|&c| c as u8).take_while(|&b| b != 0).collect();
        (!path.is_empty()).then(|| String::from_utf8_lossy(&path).into_owned())
    }
}

#[cfg(not(target_os = "macos"))]
mod os {
    use super::Info;

    // /proc/<pid>/stat: "pid (comm) state ppid …" — comm can hold spaces and parentheses, so read after the last ')'
    fn ppid(pid: i32) -> Option<i32> {
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        stat[stat.rfind(')')? + 1..].split_whitespace().nth(1)?.parse().ok()
    }

    pub fn info(pid: i32) -> Option<Info> {
        let cmdline = std::fs::read(format!("/proc/{pid}/cmdline")).unwrap_or_default();
        let args = cmdline.split(|&b| b == 0).filter(|a| !a.is_empty()).map(|a| String::from_utf8_lossy(a).into_owned()).collect::<Vec<_>>().join(" ");
        Some(Info { pid, ppid: ppid(pid)?, args })
    }

    // the kernel's own lists when it keeps them (CONFIG_PROC_CHILDREN): one per thread, since a child belongs to the
    // thread that started it; else every process's parent
    pub fn children(pid: i32) -> Vec<i32> {
        let lists: Vec<String> = std::fs::read_dir(format!("/proc/{pid}/task"))
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|t| std::fs::read_to_string(t.path().join("children")).ok())
            .collect();
        if !lists.is_empty() {
            return lists.iter().flat_map(|l| l.split_whitespace()).filter_map(|p| p.parse().ok()).collect();
        }
        std::fs::read_dir("/proc")
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| e.file_name().to_str()?.parse::<i32>().ok())
            .filter(|&p| ppid(p) == Some(pid))
            .collect()
    }

    pub fn cwd(pid: i32) -> Option<String> {
        std::fs::read_link(format!("/proc/{pid}/cwd")).ok().map(|p| p.to_string_lossy().into_owned())
    }
}

pub use os::{children, cwd, info};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn knows_this_process() {
        let me = std::process::id() as i32;
        let i = info(me).unwrap();
        assert_eq!(i.ppid, unsafe { libc::getppid() });
        assert!(i.args.contains("modisa"), "{}", i.args);
        assert_eq!(cwd(me).unwrap(), std::env::current_dir().unwrap().canonicalize().unwrap().to_string_lossy());
        let mut child = std::process::Command::new("sleep").arg("5").spawn().unwrap();
        let pid = child.id() as i32;
        assert!(children(me).contains(&pid));
        // Linux: a moment after a spawn, its command line can still be empty (the kernel sets it up late in exec)
        let args = (0..100).map(|_| info(pid).unwrap().args).find(|a| !a.is_empty() || { std::thread::sleep(std::time::Duration::from_millis(10)); false });
        assert_eq!(args.as_deref(), Some("sleep 5"));
        assert!(alive(pid));
        child.kill().unwrap();
        child.wait().unwrap();
        assert!(!alive(pid));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn reads_procargs() {
        let mut buf = 2i32.to_ne_bytes().to_vec();
        buf.extend(b"/usr/bin/codex\0\0\0codex\0--model\0LANG=C\0");
        assert_eq!(os::parse_procargs(&buf).as_deref(), Some("codex --model"));
    }
}
