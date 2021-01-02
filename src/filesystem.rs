use anyhow::{Context, Result};

use std::fs::{self, File};
use std::io::{self, ErrorKind, Read};
use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;
use serde::ser::Serialize;

use toml;

#[derive(Error, Debug)]
pub enum FileLoadError {
    #[error("open file")]
    Open(#[source] io::Error),

    #[error("read opened file")]
    Read(#[source] io::Error),

    #[error("parse file")]
    Parse(#[source] toml::de::Error),
}

pub fn load_file<T>(filename: &Path) -> Result<T, FileLoadError>
where
    T: DeserializeOwned,
{
    let mut buf = String::new();
    let mut f = File::open(filename).map_err(FileLoadError::Open)?;
    f.read_to_string(&mut buf).map_err(FileLoadError::Read)?;
    toml::from_str::<T>(&buf).map_err(FileLoadError::Parse)
}

#[derive(Error, Debug)]
pub enum FileSaveError {
    #[error("write file")]
    Write(#[source] io::Error),

    #[error("serialize data")]
    Serialize(
        #[from]
        #[source]
        toml::ser::Error,
    ),
}

pub fn save_file<T>(filename: &Path, data: T) -> Result<(), FileSaveError>
where
    T: Serialize,
{
    let data = toml::to_string(&data)?;
    fs::write(filename, &data).map_err(FileSaveError::Write)?;
    Ok(())
}

#[derive(Debug, PartialEq)]
pub enum SymlinkComparison {
    Identical,
    OnlySourceExists,
    OnlyTargetExists,
    TargetNotSymlink,
    Changed,
    BothMissing,
}

impl std::fmt::Display for SymlinkComparison {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> Result<(), std::fmt::Error> {
        use self::SymlinkComparison::*;
        match self {
            Identical => "target points at source",
            OnlySourceExists => "source exists, target missing",
            OnlyTargetExists => "source missing, target exists",
            TargetNotSymlink => "target isn't a symlink",
            Changed => "target isn't point at source",
            BothMissing => "source and target are missing",
        }
        .fmt(f)
    }
}

pub fn compare_symlink(source: &Path, link: &Path) -> Result<SymlinkComparison> {
    let source = match real_path(source) {
        Ok(s) => Some(s),
        Err(e) if e.kind() == ErrorKind::NotFound => None,
        Err(e) => Err(e).context("get canonical path of source")?,
    };

    let link_content = match fs::symlink_metadata(link) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            Some(fs::read_link(link).context("read target of link")?)
        }
        Ok(_) => return Ok(SymlinkComparison::TargetNotSymlink),
        Err(e) if e.kind() == ErrorKind::NotFound => None,
        Err(e) => Err(e).context("read metadata of link")?,
    };

    Ok(match (source, link_content) {
        (Some(s), Some(l)) => {
            if s == l {
                SymlinkComparison::Identical
            } else {
                SymlinkComparison::Changed
            }
        }
        (None, Some(_)) => SymlinkComparison::OnlyTargetExists,
        (Some(_), None) => SymlinkComparison::OnlySourceExists,
        (None, None) => SymlinkComparison::BothMissing,
    })
}

#[derive(Debug, PartialEq)]
pub enum TemplateComparison {
    Identical,
    OnlyCacheExists,
    OnlyTargetExists,
    Changed,
    BothMissing,
}

impl std::fmt::Display for TemplateComparison {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> Result<(), std::fmt::Error> {
        use self::TemplateComparison::*;
        match self {
            Identical => "target and cache's contents are equal",
            OnlyCacheExists => "cache exists, target missing",
            OnlyTargetExists => "cache missing, target exists",
            Changed => "target and cache's contents differ",
            BothMissing => "cache and target are missing",
        }
        .fmt(f)
    }
}

pub fn compare_template(target: &Path, cache: &Path) -> Result<TemplateComparison> {
    let target = match fs::read_to_string(target) {
        Ok(t) => Some(t),
        Err(e) if e.kind() == ErrorKind::NotFound => None,
        Err(e) => Err(e).context("read content of target file")?,
    };

    let cache = match fs::read_to_string(cache) {
        Ok(c) => Some(c),
        Err(e) if e.kind() == ErrorKind::NotFound => None,
        Err(e) => Err(e).context("read contents of cache file")?,
    };

    Ok(match (target, cache) {
        (Some(t), Some(c)) => {
            if t == c {
                TemplateComparison::Identical
            } else {
                TemplateComparison::Changed
            }
        }
        (Some(_), None) => TemplateComparison::OnlyTargetExists,
        (None, Some(_)) => TemplateComparison::OnlyCacheExists,
        (None, None) => TemplateComparison::BothMissing,
    })
}

pub fn real_path(path: &Path) -> Result<PathBuf, io::Error> {
    let path = std::fs::canonicalize(&path)?;
    Ok(platform_dunce(&path))
}

pub fn ask_boolean(prompt: &str) -> bool {
    let mut buf = String::from("a"); // enter the loop at least once
    while !(buf.to_lowercase().starts_with('y')
        || buf.to_lowercase().starts_with('n')
        || buf.is_empty())
    {
        eprintln!("{}", prompt);
        buf.clear();
        io::stdin()
            .read_line(&mut buf)
            .expect("Failed to read line from stdin");
    }

    // If empty defaults to no
    buf.to_lowercase().starts_with('y')
}

pub fn delete_parents(path: &Path, ask: bool) -> Result<()> {
    let mut path = path.parent().context("get parent")?;
    while path.is_dir()
        && path
            .read_dir()
            .context("read the contents of parent directory")?
            .next()
            .is_none()
    {
        if !ask
            || ask_boolean(&format!(
                "Directory at {:?} is now empty. Delete [y/N]? ",
                path
            ))
        {
            fs::remove_dir(path).context(format!("remove directory {:?}", path))?;
        }
        path = path.parent().context(format!("get parent of {:?}", path))?;
    }
    Ok(())
}

pub fn copy_permissions(source: &Path, target: &Path) -> Result<()> {
    fs::set_permissions(
        target,
        fs::metadata(source)
            .context("get metadata of source")?
            .permissions(),
    )
    .context("set metadata of target")?;
    Ok(())
}

#[cfg(windows)]
mod filesystem_impl {
    use anyhow::{Context, Result};
    use dunce;

    use std::os::windows::fs;
    use std::path::{Path, PathBuf};

    use config::UnixUser;

    pub fn make_symlink(link: &Path, target: &Path) -> Result<()> {
        Ok(fs::symlink_file(
            super::real_path(target).context("get real path of source file")?,
            link,
        )
        .context("create symlink")?)
    }

    pub fn symlinks_enabled(test_file_path: &Path) -> Result<bool> {
        debug!(
            "Testing whether symlinks are enabled on path {:?}",
            test_file_path
        );
        let _ = std::fs::remove_file(&test_file_path);
        match fs::symlink_file("test.txt", &test_file_path) {
            Ok(()) => {
                std::fs::remove_file(&test_file_path)
                    .context(format!("remove test file {:?}", test_file_path))?;
                Ok(true)
            }
            Err(e) => {
                // os error 1314: A required privilege is not held by the client.
                if e.raw_os_error() == Some(1314) {
                    Ok(false)
                } else {
                    Err(e).context(format!("create test symlink at {:?}", test_file_path))
                }
            }
        }
    }

    pub fn platform_dunce(path: &Path) -> PathBuf {
        dunce::simplified(&path).into()
    }

    pub fn set_owner(file: &Path, _owner: Option<UnixUser>) -> Result<()> {
        warn!("ignoring `owner` field on file {:?}", file);
        Ok(())
    }
}

#[cfg(unix)]
mod filesystem_impl {
    use anyhow::{Context, Result};

    use std::io::Write;
    use std::os::linux::fs::MetadataExt;
    use std::os::unix::fs;
    use std::path::{Path, PathBuf};

    use config::UnixUser;

    pub fn make_symlink(link: &Path, target: &Path) -> Result<()> {
        Ok(fs::symlink(
            super::real_path(target).context("get real path of source file")?,
            link,
        )
        .context("create symlink")?)
    }

    pub fn symlinks_enabled(_test_file_path: &Path) -> Result<bool> {
        Ok(true)
    }

    pub fn platform_dunce(path: &Path) -> PathBuf {
        path.into()
    }

    pub fn is_owned_by_user(path: &Path) -> Result<bool> {
        let file_uid = path.metadata().context("get file metadata")?.st_uid();
        let process_uid = std::path::PathBuf::from("/proc/self")
            .metadata()
            .context("get metadata of /proc/self")?
            .st_uid();
        Ok(file_uid == process_uid)
    }

    pub fn create_dir_all(path: &Path, owner: Option<UnixUser>) -> Result<()> {
        if let Some(owner) = owner {
            debug!("Creating directory {:?} from user {:?}...", path, owner);
            let success = std::process::Command::new("sudo")
                .arg("-u")
                .arg(owner.as_sudo_arg())
                .arg("mkdir")
                .arg("-p")
                .arg(path)
                .spawn()
                .context("spawn sudo mkdir")?
                .wait()
                .context("wait for sudo mkdir")?
                .success();

            ensure!(success, "sudo mkdir failed");
        } else {
            std::fs::create_dir_all(path).context("create directories")?;
        }
        Ok(())
    }

    pub fn copy_file(source: &Path, target: &Path, owner: Option<UnixUser>) -> Result<()> {
        if let Some(owner) = owner {
            debug!("Copying {:?} -> {:?} as user {:?}", source, target, owner);
            let contents = std::fs::read_to_string(source).context("read file contents")?;
            let mut child = std::process::Command::new("sudo")
                .arg("-u")
                .arg(owner.as_sudo_arg())
                .arg("sh")
                .arg("cat")
                .arg(format!(">{}", target.as_os_str().to_string_lossy()))
                .stdin(std::process::Stdio::piped())
                .spawn()
                .context("spawn sudo cat >file")?;

            // At this point we should've gone through another sudo at the mkdir step already,
            // so sudo will not ask for the password
            child
                .stdin
                .as_ref()
                .expect("has stdin")
                .write_all(contents.as_bytes())
                .context("give input to cat")?;

            let success = child.wait().context("wait for sudo cat >file")?.success();

            ensure!(success, "sudo cat >file failed");
        } else {
            std::fs::copy(source, target).context("copy file")?;
        }

        Ok(())
    }

    pub fn set_owner(file: &Path, owner: Option<UnixUser>) -> Result<()> {
        let owner = owner.unwrap_or(UnixUser::Name(
            std::env::var("USER").context("get USER env var")?,
        ));
        debug!("Setting owner of {:?} to {:?}...", file, owner);

        let success = std::process::Command::new("sudo")
            .arg("chown")
            .arg(owner.as_chown_arg())
            .arg(file)
            .spawn()
            .context("spawn sudo chown command")?
            .wait()
            .context("wait for sudo chown command")?
            .success();

        ensure!(success, "sudo chown command failed");
        Ok(())
    }

    pub fn copy_permissions(source: &Path, target: &Path, owner: Option<UnixUser>) -> Result<()> {
        if let Some(owner) = owner {
            debug!(
                "Copying permissions {:?} -> {:?} as user {:?}",
                source, target, owner
            );
            let success = std::process::Command::new("sudo")
                .arg("chmod")
                .arg("--reference")
                .arg(source)
                .arg(target)
                .spawn()
                .context("spawn sudo chmod command")?
                .wait()
                .context("wait for sudo chmod command")?
                .success();

            ensure!(success, "sudo chmod failed");
        } else {
            std::fs::set_permissions(
                target,
                source
                    .metadata()
                    .context("get source metadata")?
                    .permissions(),
            )
            .context("set target permissions")?;
        }
        Ok(())
    }
}

pub use self::filesystem_impl::*;
