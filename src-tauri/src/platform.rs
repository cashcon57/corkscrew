//! Platform-optimized file operations.
//!
//! Provides fast-path copy and hash operations that leverage platform-specific
//! features when available, with automatic fallback to standard operations:
//!
//! | Platform | FS        | Copy Method              | Hash Method              |
//! |----------|-----------|--------------------------|--------------------------|
//! | macOS    | APFS      | `clonefile()` → `fs::copy` | memmap (>1MB) → buffered |
//! | macOS    | HFS+      | `fs::copy()`             | memmap (>1MB) → buffered |
//! | Linux    | Btrfs/XFS | `ioctl(FICLONE)` → `fs::copy` | memmap (>1MB) → buffered |
//! | Linux    | ext4      | `fs::copy()`             | memmap (>1MB) → buffered |
//!
//! Detection is designed to be called once per batch operation (deploy, stage)
//! and the resulting `FsCopyMethod` reused for all files in that batch.

use std::fs;
use std::io::{self, Read, Write};
use std::path::Path;

use log::debug;
use sha2::{Digest, Sha256};

/// Files larger than 1 MiB use memory-mapped hashing; smaller files use
/// buffered reads. This threshold balances mmap overhead against throughput.
const MMAP_THRESHOLD: u64 = 1_048_576;

// ---------------------------------------------------------------------------
// Copy method enum
// ---------------------------------------------------------------------------

/// Describes which fast-copy strategy to attempt before falling back to
/// `std::fs::copy()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FsCopyMethod {
    /// macOS APFS `clonefile()` — instant copy-on-write clone.
    Clonefile,
    /// Linux Btrfs/XFS `ioctl(FICLONE)` — reflink copy-on-write.
    Reflink,
    /// Standard buffered `fs::copy()`.
    StandardCopy,
}

// ---------------------------------------------------------------------------
// Detection
// ---------------------------------------------------------------------------

/// Detect the best copy method for operations between `src_dir` and `dst_dir`.
///
/// Call once at the start of a batch operation (deploy, stage) and reuse the
/// result for every file in that batch.
///
/// On macOS the default filesystem is APFS, so we optimistically return
/// `Clonefile`. On Linux we check `/proc/mounts` for btrfs or xfs on the
/// relevant mount point. If detection fails or the platform is unsupported,
/// `StandardCopy` is returned.
pub fn detect_copy_method(src_dir: &Path, dst_dir: &Path) -> FsCopyMethod {
    detect_copy_method_impl(src_dir, dst_dir)
}

#[cfg(target_os = "macos")]
fn detect_copy_method_impl(_src_dir: &Path, _dst_dir: &Path) -> FsCopyMethod {
    // APFS is the default filesystem on all modern Macs (10.13+).
    // clonefile() will fail gracefully on HFS+ and we fall back to fs::copy().
    FsCopyMethod::Clonefile
}

#[cfg(target_os = "linux")]
fn detect_copy_method_impl(src_dir: &Path, dst_dir: &Path) -> FsCopyMethod {
    if supports_reflink(src_dir) && supports_reflink(dst_dir) {
        FsCopyMethod::Reflink
    } else {
        FsCopyMethod::StandardCopy
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn detect_copy_method_impl(_src_dir: &Path, _dst_dir: &Path) -> FsCopyMethod {
    FsCopyMethod::StandardCopy
}

/// Check whether a directory resides on a filesystem that supports reflinks
/// (btrfs or xfs) by reading `/proc/mounts`.
#[cfg(target_os = "linux")]
fn supports_reflink(dir: &Path) -> bool {
    use std::io::BufRead;

    // Canonicalize the path so we can match against mount points.
    let canonical = match dir.canonicalize() {
        Ok(p) => p,
        Err(_) => return false,
    };
    let dir_str = canonical.to_string_lossy();

    let mounts = match fs::File::open("/proc/mounts") {
        Ok(f) => f,
        Err(_) => return false,
    };

    let mut best_mount = String::new();
    let mut best_fs = String::new();

    for line in io::BufReader::new(mounts).lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => continue,
        };
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 3 {
            continue;
        }
        let mount_point = parts[1];
        let fs_type = parts[2];

        // Find the longest mount point that is a prefix of our path.
        if dir_str.starts_with(mount_point) && mount_point.len() > best_mount.len() {
            best_mount = mount_point.to_string();
            best_fs = fs_type.to_string();
        }
    }

    matches!(best_fs.as_str(), "btrfs" | "xfs")
}

// ---------------------------------------------------------------------------
// Fast copy
// ---------------------------------------------------------------------------

/// Copy a file using the best available method, falling back to `fs::copy()`
/// on any error.
///
/// For `Clonefile`/`Reflink` the destination must not exist beforehand (the
/// platform call creates it). If the destination already exists it is removed
/// before attempting the fast path.
pub fn fast_copy(src: &Path, dst: &Path, method: FsCopyMethod) -> io::Result<()> {
    match method {
        FsCopyMethod::Clonefile => {
            // Remove destination if it exists — clonefile requires a fresh path.
            if dst.exists() {
                let _ = fs::remove_file(dst);
            }
            match try_clonefile(src, dst) {
                Ok(()) => {
                    debug!("clonefile: {} -> {}", src.display(), dst.display());
                    Ok(())
                }
                Err(e) => {
                    debug!(
                        "clonefile failed ({}) for {} -> {}, falling back to fs::copy",
                        e,
                        src.display(),
                        dst.display()
                    );
                    fs::copy(src, dst)?;
                    Ok(())
                }
            }
        }
        FsCopyMethod::Reflink => {
            if dst.exists() {
                let _ = fs::remove_file(dst);
            }
            match try_reflink(src, dst) {
                Ok(()) => {
                    debug!("reflink: {} -> {}", src.display(), dst.display());
                    Ok(())
                }
                Err(e) => {
                    debug!(
                        "reflink failed ({}) for {} -> {}, falling back to fs::copy",
                        e,
                        src.display(),
                        dst.display()
                    );
                    fs::copy(src, dst)?;
                    Ok(())
                }
            }
        }
        FsCopyMethod::StandardCopy => {
            fs::copy(src, dst)?;
            Ok(())
        }
    }
}

// ---------------------------------------------------------------------------
// Fast hash
// ---------------------------------------------------------------------------

/// Compute the SHA-256 hash of a file.
///
/// Files larger than 1 MiB are memory-mapped for throughput; smaller files
/// use buffered 128 KiB reads to avoid mmap overhead.
pub fn fast_hash(path: &Path) -> io::Result<String> {
    let metadata = fs::metadata(path)?;
    if metadata.len() > MMAP_THRESHOLD {
        hash_mmap(path)
    } else {
        hash_buffered(path)
    }
}

/// Memory-mapped hashing via `memmap2`. The entire file is mapped into the
/// address space and fed to SHA-256 in one shot.
fn hash_mmap(path: &Path) -> io::Result<String> {
    let file = fs::File::open(path)?;
    // SAFETY: The file is opened read-only and we do not write through the
    // mapping. The mapping is dropped before we return, and we tolerate
    // concurrent modification (worst case: a different hash, which is fine
    // for integrity checks since the file truly changed).
    let mmap = unsafe { memmap2::Mmap::map(&file)? };
    let mut hasher = Sha256::new();
    hasher.update(&mmap[..]);
    Ok(format!("{:x}", hasher.finalize()))
}

/// Buffered 128 KiB read hashing — used for small files where mmap overhead
/// would dominate.
fn hash_buffered(path: &Path) -> io::Result<String> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 131_072]; // 128 KiB
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

// ---------------------------------------------------------------------------
// Fast copy + hash (combined)
// ---------------------------------------------------------------------------

/// Copy a file and compute its SHA-256 hash in one logical operation.
///
/// For `Clonefile` / `Reflink`: the copy is instant (CoW), then the
/// destination is hashed via `fast_hash`.
///
/// For `StandardCopy`: the file is read once and simultaneously written to
/// the destination and fed to the hasher (same single-pass pattern used by
/// the original `copy_and_hash` in staging.rs).
///
/// Returns `(sha256_hex, file_size)`.
pub fn fast_copy_and_hash(
    src: &Path,
    dst: &Path,
    method: FsCopyMethod,
) -> io::Result<(String, u64)> {
    match method {
        FsCopyMethod::Clonefile | FsCopyMethod::Reflink => {
            // Fast path: copy is ~instant via CoW, then hash the result.
            fast_copy(src, dst, method)?;
            let metadata = fs::metadata(dst)?;
            let size = metadata.len();
            let hash = fast_hash(dst)?;
            Ok((hash, size))
        }
        FsCopyMethod::StandardCopy => {
            // Single-pass: read source once, write + hash simultaneously.
            copy_and_hash_buffered(src, dst)
        }
    }
}

/// Single-pass buffered copy + hash. Reads the source once and writes each
/// chunk to both the destination file and the SHA-256 hasher.
fn copy_and_hash_buffered(src: &Path, dst: &Path) -> io::Result<(String, u64)> {
    let mut reader = fs::File::open(src)?;
    let mut writer = fs::File::create(dst)?;
    let mut hasher = Sha256::new();

    let mut buf = vec![0u8; 128 * 1024]; // 128 KiB
    let mut total: u64 = 0;

    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        writer.write_all(&buf[..n])?;
        hasher.update(&buf[..n]);
        total += n as u64;
    }

    let hash = format!("{:x}", hasher.finalize());
    Ok((hash, total))
}

// ---------------------------------------------------------------------------
// Platform-specific: macOS clonefile()
// ---------------------------------------------------------------------------

#[cfg(target_os = "macos")]
fn try_clonefile(src: &Path, dst: &Path) -> io::Result<()> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    extern "C" {
        fn clonefile(src: *const libc::c_char, dst: *const libc::c_char, flags: u32)
            -> libc::c_int;
    }

    let src_bytes = src.as_os_str().as_bytes();
    let dst_bytes = dst.as_os_str().as_bytes();

    // CString::new fails if the bytes contain an interior NUL. That would
    // indicate a corrupted path — return an error rather than panicking.
    let src_c = CString::new(src_bytes).map_err(|e| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("source path contains NUL byte: {}", e),
        )
    })?;
    let dst_c = CString::new(dst_bytes).map_err(|e| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("destination path contains NUL byte: {}", e),
        )
    })?;

    let ret = unsafe { clonefile(src_c.as_ptr(), dst_c.as_ptr(), 0) };
    if ret == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(not(target_os = "macos"))]
fn try_clonefile(_src: &Path, _dst: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "clonefile is only available on macOS",
    ))
}

// ---------------------------------------------------------------------------
// Platform-specific: Linux ioctl(FICLONE)
// ---------------------------------------------------------------------------

#[cfg(target_os = "linux")]
fn try_reflink(src: &Path, dst: &Path) -> io::Result<()> {
    use std::os::unix::io::AsRawFd;

    // FICLONE ioctl number — defined in linux/fs.h
    const FICLONE: libc::c_ulong = 0x40049409;

    let src_file = fs::File::open(src)?;
    let dst_file = fs::File::create(dst)?;

    let ret = unsafe { libc::ioctl(dst_file.as_raw_fd(), FICLONE, src_file.as_raw_fd()) };
    if ret == 0 {
        Ok(())
    } else {
        // Clean up the empty destination file on failure.
        drop(dst_file);
        let _ = fs::remove_file(dst);
        Err(io::Error::last_os_error())
    }
}

#[cfg(not(target_os = "linux"))]
fn try_reflink(_src: &Path, _dst: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "reflink (FICLONE) is only available on Linux",
    ))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn fast_hash_matches_known_sha256() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("hello.txt");
        fs::write(&path, b"hello world").unwrap();

        let hash = fast_hash(&path).unwrap();
        assert_eq!(
            hash,
            "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"
        );
    }

    #[test]
    fn fast_hash_empty_file() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("empty.txt");
        fs::write(&path, b"").unwrap();

        let hash = fast_hash(&path).unwrap();
        // SHA-256 of empty input
        assert_eq!(
            hash,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn fast_hash_large_file_uses_mmap_path() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("big.bin");
        // Create a file just above the mmap threshold.
        let data = vec![0xABu8; (MMAP_THRESHOLD + 1) as usize];
        fs::write(&path, &data).unwrap();

        let hash = fast_hash(&path).unwrap();
        assert!(!hash.is_empty());
        assert_eq!(hash.len(), 64); // SHA-256 hex is 64 chars

        // Verify it matches the buffered path.
        let buffered = hash_buffered(&path).unwrap();
        assert_eq!(hash, buffered);
    }

    #[test]
    fn fast_copy_standard_works() {
        let tmp = TempDir::new().unwrap();
        let src = tmp.path().join("src.txt");
        let dst = tmp.path().join("dst.txt");
        fs::write(&src, b"test data").unwrap();

        fast_copy(&src, &dst, FsCopyMethod::StandardCopy).unwrap();
        assert_eq!(fs::read(&dst).unwrap(), b"test data");
    }

    #[test]
    fn fast_copy_and_hash_standard_works() {
        let tmp = TempDir::new().unwrap();
        let src = tmp.path().join("src.txt");
        let dst = tmp.path().join("dst.txt");
        fs::write(&src, b"hello world").unwrap();

        let (hash, size) = fast_copy_and_hash(&src, &dst, FsCopyMethod::StandardCopy).unwrap();
        assert_eq!(fs::read(&dst).unwrap(), b"hello world");
        assert_eq!(size, 11);
        assert_eq!(
            hash,
            "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"
        );
    }

    #[test]
    fn detect_copy_method_returns_valid_variant() {
        let tmp = TempDir::new().unwrap();
        let a = tmp.path().join("a");
        let b = tmp.path().join("b");
        fs::create_dir_all(&a).unwrap();
        fs::create_dir_all(&b).unwrap();

        let method = detect_copy_method(&a, &b);
        // On any platform, we should get a valid variant.
        assert!(matches!(
            method,
            FsCopyMethod::Clonefile | FsCopyMethod::Reflink | FsCopyMethod::StandardCopy
        ));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn clonefile_on_same_volume() {
        let tmp = TempDir::new().unwrap();
        let src = tmp.path().join("clone_src.txt");
        let dst = tmp.path().join("clone_dst.txt");
        fs::write(&src, b"clone me").unwrap();

        // On APFS this should succeed; on HFS+ it will fail and that's fine
        // because fast_copy handles the fallback.
        let method = detect_copy_method(tmp.path(), tmp.path());
        fast_copy(&src, &dst, method).unwrap();
        assert_eq!(fs::read(&dst).unwrap(), b"clone me");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn fast_copy_and_hash_clonefile() {
        let tmp = TempDir::new().unwrap();
        let src = tmp.path().join("clone_hash_src.txt");
        let dst = tmp.path().join("clone_hash_dst.txt");
        fs::write(&src, b"hello world").unwrap();

        let method = detect_copy_method(tmp.path(), tmp.path());
        let (hash, size) = fast_copy_and_hash(&src, &dst, method).unwrap();
        assert_eq!(fs::read(&dst).unwrap(), b"hello world");
        assert_eq!(size, 11);
        assert_eq!(
            hash,
            "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"
        );
    }
}
