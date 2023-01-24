use std::{
    collections::hash_map::DefaultHasher,
    env, fs,
    hash::{Hash, Hasher},
    path::PathBuf,
    str::FromStr,
};

use proc_macro::TokenStream;

/// Includes a binary file that was prepared for inclusion by a build script.
///
/// Takes a string literal as its argument, denoting the file's path (relative to the directory
/// containing the package's `Cargo.toml`).
///
/// The macro expands to an expression of type `&[u8; N]`, where `N` is the size of the file in
/// Bytes.
#[proc_macro]
pub fn include_blob(args: TokenStream) -> TokenStream {
    let lit: syn::LitStr = syn::parse(args).unwrap();
    let lit = lit.value();

    let mut path = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    path.push(lit);

    let path = path.canonicalize().unwrap_or_else(|_| {
        panic!("could not find file '{}'", path.display(),);
    });
    let metadata = fs::metadata(&path).unwrap();
    assert!(metadata.is_file());
    let len = metadata.len();

    let mut hasher = DefaultHasher::new();
    path.hash(&mut hasher);
    metadata.modified().unwrap().hash(&mut hasher);
    let unique_name = format!("include_blob_{:016x}", hasher.finish());

    TokenStream::from_str(&format!(
        r#"
        {{
            extern "C" {{
                #[link_name = "{unique_name}"]
                static STATIC: [u8; {len}];
            }}
            unsafe {{ &STATIC }}
        }}
        "#
    ))
    .unwrap()
}
