#[path = "../../../std/bindings/Crypto.rs"]
mod crypto;
#[path = "../../../std/bindings/Fs.rs"]
mod fs;
#[path = "../../../std/bindings/Process.rs"]
mod process;
#[path = "../../../std/bindings/Regex.rs"]
mod regex;
#[path = "../../../std/bindings/Test.rs"]
mod test;
#[path = "../../../std/bindings/Time.rs"]
mod time;
#[path = "../../../std/bindings/Url.rs"]
mod url;

use crate::value::NativeFn;

#[must_use]
pub fn lookup(key: &str) -> Option<NativeFn> {
  Some(match key {
    "url::decode" => url::decode,
    "url::encode" => url::encode,
    "time::now" => time::now,
    "time::to_parts" => time::to_parts,
    "time::from_parts" => time::from_parts,
    "crypto::sha256" => crypto::sha256,
    "crypto::hmac_sha256" => crypto::hmac_sha256,
    "crypto::equal" => crypto::equal,
    "crypto::base64url" => crypto::base64url,
    "crypto::from_base64url" => crypto::from_base64url,
    "crypto::scrypt" => crypto::scrypt,
    "crypto::try_scrypt" => crypto::try_scrypt,
    "crypto::bytes" => crypto::bytes,
    "regex::matches" => regex::matches,
    "regex::first_match" => regex::first_match,
    "regex::all_matches" => regex::all_matches,
    "regex::capture_groups" => regex::capture_groups,
    "regex::replace_all" => regex::replace_all,
    "regex::syntax_error" => regex::syntax_error,
    "fs::read" => fs::read,
    "fs::write" => fs::write,
    "fs::append" => fs::append,
    "fs::exists" => fs::exists,
    "fs::is_dir" => fs::is_dir,
    "fs::mkdir_all" => fs::mkdir_all,
    "fs::list" => fs::list,
    "fs::walk" => fs::walk,
    "fs::remove" => fs::remove,
    "fs::remove_all" => fs::remove_all,
    "fs::mkdtemp" => fs::mkdtemp,
    "fs::with_temp_dir" => fs::with_temp_dir,
    "process::args" => process::args,
    "process::env" => process::env,
    "process::cwd" => process::cwd,
    "process::set_exit_code" => process::set_exit_code,
    "process::run" => process::run,
    "process::output" => process::output,
    "process::scoped_env" => process::scoped_env,
    "test::capture" => test::capture,
    "test::guard" => test::guard,
    "test::compare_snapshot" => test::compare_snapshot,
    "test::clock" => test::clock,
    "test::finish" => test::finish,
    "test::mock" => test::mock,
    _ => return None,
  })
}
