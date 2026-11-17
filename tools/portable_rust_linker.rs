//! GCC-compatible *link* driver for the packaged Rust GNU target. This never
//! compiles C/C++: a project requiring a native compiler must configure one.
//! All arguments remain separate OS strings; no shell is involved.
use std::{
    env,
    ffi::OsString,
    path::PathBuf,
    process::{Command, ExitCode},
};
fn run() -> Result<i32, String> {
    let root = PathBuf::from(
        env::var_os("CODISSEU_RUST_LINKER_ROOT").ok_or("Missing private Rust linker root")?,
    );
    let bundle = PathBuf::from(
        env::var_os("CODISSEU_RUST_BUNDLE_ROOT").ok_or("Missing private Rust bundle root")?,
    );
    let lld = root.join("../toolchain/lib/rustlib/x86_64-unknown-linux-gnu/bin/rust-lld");
    let crt = root.join("crt");
    let libs = root.join("lib");
    let mut output = Vec::<OsString>::new();
    let mut shared = false;
    let mut no_start = false;
    let mut is_static = false;
    let mut args = env::args_os().skip(1).peekable();
    while let Some(arg) = args.next() {
        let text = arg.to_string_lossy();
        match text.as_ref() {
            "-c" | "-E" | "-S" => return Err("The portable Rust linker cannot compile C/C++. Configure the native compiler required by this project (for example CC=/usr/bin/cc).".into()),
            "--version" | "-v" if output.is_empty() && args.peek().is_none() => return Ok(Command::new(&lld).arg("--version").status().map_err(|e| e.to_string())?.code().unwrap_or(1)),
            "-m64" | "-nodefaultlibs" | "-fuse-ld=lld" => {},
            "-m32" | "-static-pie" => return Err("The private Rust linker supports the Linux x86_64 GNU dynamic target; configure an explicit linker for this mode".into()),
            "-nostdlib" | "-nostartfiles" => no_start = true,
            "-static" => { is_static = true; output.push(arg); },
            "-shared" => { shared = true; output.push(arg); },
            "-pthread" => output.push("-lpthread".into()),
            "-rdynamic" => output.push("--export-dynamic".into()),
            "-Xlinker" => output.push(args.next().ok_or("-Xlinker requires an argument")?),
            "-o" | "-L" | "-l" | "-T" | "-u" => { output.push(arg); output.push(args.next().ok_or("Link option requires an argument")?); },
            _ if text.starts_with("-Wl,") => output.extend(text[4..].split(',').map(OsString::from)),
            _ if text.starts_with("-B") => {}, // GCC's program search path; we bind our exact linker above.
            _ if text.starts_with("-fuse-ld=") => return Err("Configure the Cargo target linker explicitly to select another linker".into()),
            _ if text.starts_with("-f") || text.starts_with("-D") || text.starts_with("-I") => return Err(format!("C compiler option {text} is not supported by the portable Rust link driver; configure a native compiler")),
            _ => output.push(arg),
        }
    }
    if is_static {
        return Err("Static libc linking requires an explicit native toolchain; the bundled Rust linker targets private dynamic glibc".into());
    }
    let mut command = Command::new(&lld);
    command.args(["-flavor", "gnu", "-m", "elf_x86_64"]);
    command
        .arg("-L")
        .arg(&libs)
        .arg("-L")
        .arg(bundle.join("lib"));
    if !shared {
        command
            .arg("--dynamic-linker")
            .arg(bundle.join("lib/ld-linux-x86-64.so.2"));
        command.arg("-rpath").arg(bundle.join("lib"));
    }
    if !no_start {
        if !shared {
            command.arg(crt.join("Scrt1.o"));
        }
        command.arg(crt.join("crti.o")).arg(crt.join("crtbeginS.o"));
    }
    command.args(output);
    if !no_start {
        command.arg(crt.join("crtendS.o")).arg(crt.join("crtn.o"));
    }
    let status = command.status().map_err(|e| {
        format!(
            "Cannot start the private Rust linker {}: {e}",
            lld.display()
        )
    })?;
    Ok(status.code().unwrap_or(1))
}
fn main() -> ExitCode {
    match run() {
        Ok(code) => ExitCode::from(u8::try_from(code).unwrap_or(1)),
        Err(error) => {
            eprintln!("Codisseu Rust linker: {error}");
            ExitCode::FAILURE
        }
    }
}
