pub use include_blob_macros::*;

use std::{
    collections::{hash_map::DefaultHasher, BTreeMap},
    env, error,
    fs::{self, File},
    hash::{Hash, Hasher},
    io::{Seek, Write},
    path::{Path, PathBuf},
    process::Command,
};

use object::{
    write::{Object, StandardSection, Symbol, SymbolSection},
    Architecture, BinaryFormat, Endianness, SymbolFlags, SymbolKind, SymbolScope,
};

type Result<T> = std::result::Result<T, Box<dyn error::Error>>;

/// Call this from your build script to make `path` includable via `include_blob!`.
///
/// `path` can refer to a file or a directory (which processes every file in the directory).
///
/// `path` is relative to the directory the build script runs in (which is the package's "source
/// directory" according to Cargo's docs, so probably the directory containing `Cargo.toml`).
pub fn make_includable<A: AsRef<Path>>(path: A) {
    make_includable_impl(path.as_ref()).unwrap();
}

fn make_includable_impl(path: &Path) -> Result<()> {
    let path = path.canonicalize().unwrap_or_else(|_| {
        panic!(
            "could not find file '{}' (working directory is '{}')",
            path.display(),
            std::env::current_dir().unwrap().display(),
        );
    });
    let metadata = fs::metadata(&path)?;

    if metadata.is_dir() {
        for entry in fs::read_dir(&path)? {
            let entry = entry?;
            make_includable_impl(&entry.path())?;
        }
        Ok(())
    } else if metadata.is_file() {
        process_file(path, metadata)
    } else {
        panic!(
            "cannot handle file type '{:?}' of '{}'",
            metadata.file_type(),
            path.display()
        );
    }
}

fn process_file(path: PathBuf, metadata: fs::Metadata) -> Result<()> {
    let mut hasher = DefaultHasher::new();
    path.hash(&mut hasher);
    metadata.modified()?.hash(&mut hasher);
    let unique_name = format!("include_blob_{:016x}", hasher.finish());

    let content = fs::read(&path)?;

    let (pre, post) = lib_prefix_and_suffix();
    let out_dir = env::var("OUT_DIR")?;
    let out_file_path = format!("{out_dir}/{pre}{unique_name}{post}");
    let mut out_file = File::create(&out_file_path)?;

    let info = TargetInfo::from_build_script_vars();
    let mut obj_buf = Vec::new();
    let mut object = Object::new(info.binfmt, info.arch, info.endian);
    let (section, _) = object.add_subsection(
        StandardSection::ReadOnlyData,
        unique_name.as_bytes(),
        &[],
        1,
    );
    let symbol_name = unique_name.as_bytes().to_vec();
    let sym = object.add_symbol(Symbol {
        name: symbol_name.clone(),
        value: 0,
        size: content.len() as _,
        kind: SymbolKind::Data,
        scope: SymbolScope::Linkage,
        weak: false,
        section: SymbolSection::Section(section),
        flags: SymbolFlags::None,
    });
    object.add_symbol_data(sym, section, &content, 1);
    object.write_stream(&mut obj_buf)?;

    let object_file_name = format!("{unique_name}.o").into_bytes();
    write_archive(&mut out_file, &object_file_name, &obj_buf, &symbol_name)?;

    if !use_gnu_archive() {
        // builtin ranlib doesn't currently work on macOS, run external ranlib command
        let out = Command::new("ranlib").arg(out_file_path).output()?;
        if !out.status.success() {
            panic!(
                "failed to run ranlib: {:?}\nstderr:\n{}",
                out.status,
                String::from_utf8_lossy(&out.stderr)
            );
        }
    }

    println!("cargo:rustc-link-lib=static={unique_name}");
    println!("cargo:rustc-link-search=native={out_dir}");
    println!("cargo:rerun-if-changed={}", path.display());
    Ok(())
}

fn write_archive(
    out_file: &mut (impl Write + Seek),
    object_file_name: &[u8],
    object_file_contents: &[u8],
    symbol_name: &[u8],
) -> Result<()> {
    let mut symtab = BTreeMap::new();
    symtab.insert(object_file_name.to_vec(), vec![symbol_name.to_vec()]);

    if dbg!(use_gnu_archive()) {
        ar::GnuBuilder::new(
            out_file,
            vec![object_file_name.to_vec()],
            ar::GnuSymbolTableFormat::Size32,
            symtab,
        )?
        .append(
            &ar::Header::new(object_file_name.to_vec(), object_file_contents.len() as u64),
            &object_file_contents[..],
        )?;
    } else {
        ar::Builder::new(out_file, BTreeMap::new())?.append(
            &ar::Header::new(object_file_name.to_vec(), object_file_contents.len() as u64),
            &object_file_contents[..],
        )?;
    }
    Ok(())
}

struct TargetInfo {
    binfmt: BinaryFormat,
    arch: Architecture,
    endian: Endianness,
}

impl TargetInfo {
    fn from_build_script_vars() -> Self {
        let binfmt = match &*env::var("CARGO_CFG_TARGET_OS").unwrap() {
            "macos" | "ios" => BinaryFormat::MachO,
            "windows" => BinaryFormat::Coff,
            "linux" | "android" => BinaryFormat::Elf,
            unk => panic!("unhandled operating system '{unk}'"),
        };
        let arch = match &*env::var("CARGO_CFG_TARGET_ARCH").unwrap() {
            // NB: this is guesswork, because apparently the Rust team can't be bothered to document
            // the *full* list anywhere (they differ from what the target triples use, which *are*
            // fully documented)
            "x86" => Architecture::I386,
            "x86_64" => Architecture::X86_64,
            "arm" => Architecture::Arm,
            "aarch64" => Architecture::Aarch64,
            "riscv32" => Architecture::Riscv32,
            "riscv64" => Architecture::Riscv64,
            "mips" => Architecture::Mips,
            "mips64" => Architecture::Mips64,
            "powerpc" => Architecture::PowerPc,
            "powerpc64" => Architecture::PowerPc64,
            unk => panic!("unhandled architecture '{unk}'"),
        };
        let endian = match &*env::var("CARGO_CFG_TARGET_ENDIAN").unwrap() {
            "little" => Endianness::Little,
            "big" => Endianness::Big,
            unk => unreachable!("unhandled endianness '{unk}'"),
        };

        Self {
            binfmt,
            arch,
            endian,
        }
    }
}

fn lib_prefix_and_suffix() -> (&'static str, &'static str) {
    if env::var_os("CARGO_CFG_UNIX").is_some() {
        ("lib", ".a")
    } else if env::var_os("CARGO_CFG_WINDOWS").is_some() {
        ("", ".lib")
    } else {
        unimplemented!("target platform not supported");
    }
}

fn use_gnu_archive() -> bool {
    &*env::var("CARGO_CFG_TARGET_VENDOR").unwrap() != "apple"
}
