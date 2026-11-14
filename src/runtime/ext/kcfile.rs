//! The "File" object (kcfile.mfx): file and folder operations, through the save sandbox.
//!
//! Implemented (indices inferred from how a game's launcher uses them, by parameter and
//! purpose): condition 4 file exists, condition 5 folder exists; action 2 create folder,
//! action 4 create (empty) file; expression 27 the user's Documents folder.

use super::{arg_s, Extension};
use crate::ctf_loader::events::Comparison;
use crate::runtime::files::SaveFs;
use crate::runtime::value::Value;

pub struct FileObject;

/// The user's Documents folder, without a trailing separator.
fn documents() -> String {
    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).map(std::path::PathBuf::from).unwrap_or_default();
    home.join("Documents").to_string_lossy().trim_end_matches(['\\', '/']).to_string()
}

impl Extension for FileObject {
    fn condition(&self, idx: usize, args: &[Value], _cmp: Comparison, fs: &SaveFs) -> Option<bool> {
        let path = arg_s(args, 0);
        let path = path.trim_end_matches(['\\', '/']);
        match idx {
            4 => Some(fs.exists(path)),
            5 => Some(fs.dir_exists(path)),
            _ => None,
        }
    }

    fn action(&mut self, idx: usize, args: &[Value], fs: &SaveFs) -> bool {
        let path = arg_s(args, 0);
        match idx {
            2 => {
                if let Err(e) = fs.create_dir(&path) {
                    eprintln!("File object: cannot create folder {path}: {e}");
                }
            }
            4 => {
                if !fs.exists(&path) {
                    if let Err(e) = fs.write(&path, b"") {
                        eprintln!("File object: cannot create {path}: {e}");
                    }
                }
            }
            _ => return false,
        }
        true
    }

    fn expression(&self, idx: usize, _args: &[Value], _fs: &SaveFs) -> Option<Value> {
        match idx {
            27 => Some(Value::Str(documents())),
            _ => None,
        }
    }
}
