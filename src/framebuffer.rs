//! Read the registered Paper Pro framebuffer when XOVI is active. UI extensions
//! change heap layout, so the stock allocator heuristic is not reliable there.
use anyhow::{ensure, Context, Result};
use image::ImageEncoder;
use std::{
    fs::OpenOptions,
    io::{Read, Seek, SeekFrom, Write},
    os::unix::{fs::OpenOptionsExt, io::AsRawFd},
    sync::Mutex,
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Config {
    address: u64,
    width: u32,
    height: u32,
    stride: usize,
}

fn config(text: &str) -> Result<Config> {
    let fields: Vec<_> = text.trim().split(',').collect();
    ensure!(fields.len() == 6, "Framebuffer metadata unavailable");
    let address = u64::from_str_radix(fields[0].strip_prefix("0x").context("Invalid framebuffer pointer")?, 16)?;
    ensure!(address > 4096 && fields[3] == "2" && fields[5] == "0", "Unsupported framebuffer format");
    let (width, height, stride) = match (fields[1], fields[2], fields[4]) {
        ("1620", "2160", "6528") => (1620, 2160, 6528),
        ("960", "1696", "3840") => (960, 1696, 3840),
        _ => anyhow::bail!("Unsupported framebuffer geometry"),
    };
    Ok(Config { address, width, height, stride })
}

fn packed_rgba(raw: &[u8], width: usize, stride: usize) -> Result<Vec<u8>> {
    ensure!(stride >= width * 4 && raw.len() % stride == 0, "Invalid framebuffer rows");
    let mut packed = Vec::with_capacity(raw.len() / stride * width * 4);
    for row in raw.chunks_exact(stride) {
        for p in row[..width * 4].chunks_exact(4) {
            packed.extend_from_slice(&[p[2], p[1], p[0], 255]);
        }
    }
    Ok(packed)
}

#[derive(Default)]
struct CachedConfig {
    process: String,
    value: Option<Config>,
}
impl CachedConfig {
    fn get(&mut self, process: &str, read: impl FnOnce() -> Result<Config>) -> Result<Config> {
        if self.process == process {
            if let Some(value) = self.value {
                return Ok(value);
            }
        }
        let value = read()?;
        self.process = process.into();
        self.value = Some(value);
        Ok(value)
    }
}
static CACHE: Mutex<CachedConfig> = Mutex::new(CachedConfig {
    process: String::new(),
    value: None,
});

fn query_config() -> Result<Config> {
    // Serialize our capture processes because XOVI has one shared reply FIFO.
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open("/run/smart-remarkable-framebuffer.lock")?;
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        // The file stays alive through the transaction; closing releases flock.
        if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
            break;
        }
        ensure!(Instant::now() < deadline, "Framebuffer reader is busy");
        std::thread::sleep(Duration::from_millis(10));
    }
    let mut output = OpenOptions::new().read(true).custom_flags(libc::O_NONBLOCK).open("/run/xovi-mb-out")?;
    let mut input = OpenOptions::new().write(true).custom_flags(libc::O_NONBLOCK).open("/run/xovi-mb")?;
    input.write_all(b">eframebuffer-spy$getConfigString:\n")?;
    drop(input);
    let mut bytes = Vec::new();
    let mut chunk = [0u8; 256];
    loop {
        match output.read(&mut chunk) {
            Ok(0) if !bytes.is_empty() => break,
            Ok(n) if n > 0 => {
                bytes.extend_from_slice(&chunk[..n]);
                ensure!(bytes.len() <= 256, "Oversized framebuffer metadata");
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(e) => return Err(e.into()),
        }
        ensure!(Instant::now() < deadline, "Framebuffer metadata timed out");
        std::thread::sleep(Duration::from_millis(5));
    }
    config(std::str::from_utf8(&bytes)?)
}

pub fn capture_png(pid: &str) -> Result<Option<Vec<u8>>> {
    let maps = std::fs::read_to_string(format!("/proc/{pid}/maps"))?;
    if !maps.contains("framebuffer-spy.so") {
        return Ok(None);
    }
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat"))?;
    let start = stat
        .rsplit_once(") ")
        .and_then(|(_, tail)| tail.split_whitespace().nth(19))
        .context("Framebuffer process start time unavailable")?;
    // The module registers one stable buffer per xochitl process. Query once,
    // including process start time so a reused PID never reuses an old address.
    let process = format!("{pid}:{start}");
    let Config { address, width, height, stride } = CACHE
        .lock()
        .map_err(|_| anyhow::anyhow!("Framebuffer cache unavailable"))?
        .get(&process, query_config)?;
    let mut raw = vec![0; stride * height as usize];
    let mut memory = std::fs::File::open(format!("/proc/{pid}/mem"))?;
    memory.seek(SeekFrom::Start(address))?;
    memory.read_exact(&mut raw)?;
    let rgba = packed_rgba(&raw, width as usize, stride)?;
    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png).write_image(&rgba, width, height, image::ExtendedColorType::Rgba8)?;
    Ok(Some(png))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn metadata_is_reused_only_for_the_same_process_lifetime() {
        let mut cache = CachedConfig::default();
        let initial = config("0x12340000,1620,2160,2,6528,0").unwrap();
        let changed = config("0x56780000,960,1696,2,3840,0").unwrap();
        assert_eq!(cache.get("123:1", || Ok(initial)).unwrap(), initial);
        assert_eq!(cache.get("123:1", || panic!("must not query twice")).unwrap(), initial);
        assert_eq!(cache.get("123:2", || Ok(changed)).unwrap(), changed);
    }
    #[test]
    fn validates_actual_geometry_and_removes_row_padding_without_shifting_pixels() {
        assert_eq!(config("0x12340000,1620,2160,2,6528,0").unwrap(), Config { address: 0x12340000, width: 1620, height: 2160, stride: 6528 });
        assert_eq!(config("0x12340000,960,1696,2,3840,0").unwrap(), Config { address: 0x12340000, width: 960, height: 1696, stride: 3840 });
        for value in [
            "NULL",
            "0x0,1620,2160,2,6528,0",
            "0x12340000,960,1696,2,6528,0",
            "0x12340000,960,1696,1,3840,0",
            "0x12340000,1620,2160,2,6528,1",
        ] {
            assert!(config(value).is_err());
        }
        assert_eq!(
            packed_rgba(&[3, 2, 1, 0, 99, 99, 99, 99, 6, 5, 4, 0, 99, 99, 99, 99], 1, 8).unwrap(),
            vec![1, 2, 3, 255, 4, 5, 6, 255]
        );
    }
}
