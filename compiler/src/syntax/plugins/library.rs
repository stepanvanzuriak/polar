#![allow(unsafe_code)]

use polar_plugin::{ABI, Request, Response};

type AbiFn = unsafe extern "C" fn() -> u32;
type CallFn = unsafe extern "C" fn(*const u8, usize, *mut usize) -> *mut u8;
type FreeFn = unsafe extern "C" fn(*mut u8, usize);

pub(super) struct Library {
  pub(super) path: String,
  library: libloading::Library,
}

impl Library {
  pub(super) fn open(path: &str) -> Result<Self, String> {
    let library = unsafe { libloading::Library::new(path) }
      .map_err(|e| format!("cannot load the plugin `{path}`: {e}"))?;
    let abi =
      unsafe { library.get::<AbiFn>(b"polar_plugin_abi\0") }.map_err(|_| {
        format!("`{path}` is not a Polar plugin: it has no `polar_plugin_abi`")
      })?;
    let version = unsafe { abi() };

    if version != ABI {
      return Err(format!(
        "`{path}` speaks plugin ABI {version}, but this compiler speaks {ABI}: \
         rebuild it against this compiler's `polar-plugin`"
      ));
    }

    Ok(Self { path: path.to_string(), library })
  }

  pub(super) fn call(&self, request: &Request) -> Result<Response, String> {
    let bytes = serde_json::to_vec(request).map_err(|e| e.to_string())?;
    let missing = |name: &str| format!("`{}` has no `{name}`", self.path);
    let call = unsafe { self.library.get::<CallFn>(b"polar_plugin_call\0") }
      .map_err(|_| missing("polar_plugin_call"))?;
    let free = unsafe { self.library.get::<FreeFn>(b"polar_plugin_free\0") }
      .map_err(|_| missing("polar_plugin_free"))?;
    let mut len = 0usize;
    let reply = unsafe { call(bytes.as_ptr(), bytes.len(), &raw mut len) };

    if reply.is_null() {
      return Err(format!("`{}` returned nothing", self.path));
    }

    let copied = unsafe { std::slice::from_raw_parts(reply, len) }.to_vec();

    unsafe { free(reply, len) };

    serde_json::from_slice(&copied)
      .map_err(|e| format!("`{}` sent a malformed reply: {e}", self.path))
  }
}
