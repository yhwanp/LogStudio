//! Explicit CLI grants; paths never arrive through frontend IPC.
use crate::source::error;
use std::{ffi::OsString, io, path::PathBuf};
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Launch {
    /// Files to open as one combined log, in argument order.
    pub files: Vec<PathBuf>,
}
impl Launch {
    /// Options, or plain paths as the operating system passes them for
    /// "Open with" and for files dropped on the executable. Several files open
    /// as one combined log.
    pub fn parse(args: impl IntoIterator<Item = OsString>) -> io::Result<Self> {
        let mut args = args.into_iter();
        let mut files = Vec::new();
        while let Some(arg) = args.next() {
            match arg.to_str() {
                Some("--large-log") => {
                    let path = args
                        .next()
                        .filter(|p| !p.is_empty())
                        .ok_or_else(|| error("--large-log requires a file"))?;
                    files.push(PathBuf::from(path));
                }
                // Older macOS passes a process serial number to bundled apps.
                Some(s) if s.starts_with("-psn_") => {}
                _ if !arg.is_empty() && !arg.to_string_lossy().starts_with('-') => {
                    files.push(PathBuf::from(arg));
                }
                _ => return Err(error("Supported options: FILE, --large-log FILE")),
            }
        }
        if files.len() > crate::source::MAX_FILES {
            return Err(error(format!(
                "Select 1 to {} log files",
                crate::source::MAX_FILES
            )));
        }
        Ok(Self { files })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn parse(a: &[&str]) -> io::Result<Launch> {
        Launch::parse(a.iter().map(OsString::from))
    }
    #[test]
    fn no_arguments_open_nothing() {
        assert_eq!(parse(&[]).unwrap(), Launch::default());
        assert_eq!(
            parse(&["--large-log", "/synthetic 2GiB.log"]).unwrap(),
            Launch {
                files: vec![PathBuf::from("/synthetic 2GiB.log")]
            }
        );
    }
    #[test]
    fn a_plain_path_opens() {
        let path = r"C:\Logs\-v year\main log.txt";
        assert_eq!(
            parse(&[path]).unwrap(),
            Launch {
                files: vec![PathBuf::from(path)]
            }
        );
        assert_eq!(
            parse(&["-psn_0_12345", "/logs/a.log"]).unwrap().files,
            [PathBuf::from("/logs/a.log")]
        );
    }
    #[test]
    fn several_paths_open_together_in_order() {
        for a in [
            &["b.log", "a.log"][..],
            &["--large-log", "b.log", "--large-log", "a.log"],
            &["--large-log", "b.log", "a.log"],
        ] {
            assert_eq!(
                parse(a).unwrap(),
                Launch {
                    files: vec![PathBuf::from("b.log"), PathBuf::from("a.log")]
                },
                "{a:?}"
            );
        }
        let many: Vec<String> = (0..=crate::source::MAX_FILES)
            .map(|n| format!("{n}.log"))
            .collect();
        assert!(Launch::parse(many.iter().map(OsString::from)).is_err());
    }
    #[test]
    fn unsupported_and_missing() {
        for a in [
            &["--source", "ram", "--large-log", "file"][..],
            &["--source", "file", "--large-log", "file"],
            &["--ram-budget-mib", "2304", "--large-log", "file"],
            &["--ram-source", "file"],
            &["--large-log"],
            &["-x"],
            &[""],
        ] {
            assert!(parse(a).is_err(), "{a:?}");
        }
    }
}
