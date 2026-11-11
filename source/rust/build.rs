use std::{env, fs, path::PathBuf, process::Command};
fn main() {
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let icon = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap())
        .join("../../assets/icons/AirBridge.ico")
        .canonicalize()
        .unwrap();
    println!("cargo:rerun-if-changed={}", icon.display());
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let rc = out.join("airbridge.rc");
    let res = out.join("airbridge.res");
    let path = icon
        .to_string_lossy()
        .trim_start_matches(r"\\?\")
        .replace('\\', "/");
    fs::write(
        &rc,
        format!(
            r#"1 ICON "{path}"
1 VERSIONINFO
FILEVERSION 0,1,0,0
PRODUCTVERSION 0,1,0,0
FILEOS 0x40004L
FILETYPE 0x1L
BEGIN
 BLOCK "StringFileInfo"
 BEGIN
  BLOCK "040904B0"
  BEGIN
   VALUE "FileDescription", "AirBridge screen mirroring receiver\0"
   VALUE "ProductName", "AirBridge\0"
   VALUE "OriginalFilename", "AirBridge.exe\0"
   VALUE "FileVersion", "0.1.0\0"
   VALUE "ProductVersion", "0.1.0\0"
  END
 END
 BLOCK "VarFileInfo"
 BEGIN
  VALUE "Translation", 0x409, 1200
 END
END
"#
        ),
    )
    .unwrap();
    let compiler = env::var_os("RC").map(PathBuf::from).unwrap_or_else(|| {
        let base = PathBuf::from(
            env::var_os("ProgramFiles(x86)").unwrap_or_else(|| r"C:\Program Files (x86)".into()),
        )
        .join("Windows Kits/10/bin");
        let mut versions = fs::read_dir(&base)
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .map(|e| e.path())
            .collect::<Vec<_>>();
        versions.sort();
        versions.reverse();
        versions
            .into_iter()
            .map(|p| p.join("x64/rc.exe"))
            .find(|p| p.is_file())
            .unwrap_or_else(|| PathBuf::from("rc.exe"))
    });
    let status = Command::new(compiler)
        .arg("/nologo")
        .arg("/fo")
        .arg(&res)
        .arg(&rc)
        .status()
        .expect("Windows SDK rc.exe is required to embed the application icon");
    assert!(status.success(), "application resource compilation failed");
    println!("cargo:rustc-link-arg-bin=AirBridge={}", res.display());
}
