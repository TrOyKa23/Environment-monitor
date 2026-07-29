use std::env;
use std::fs::File;
use std::io::Write;
use std::path::PathBuf;

fn main() {
    // Копируем memory.x в OUT_DIR, чтобы линкер точно его нашёл
    let out = &PathBuf::from(env::var_os("OUT_DIR").unwrap());
    File::create(out.join("memory.x"))
        .unwrap()
        .write_all(include_bytes!("memory.x"))
        .unwrap();
    println!("cargo:rustc-link-search={}", out.display());
    println!("cargo:rerun-if-changed=memory.x");

    // -Tlink.x, --nmagic и (для rp2040) -Tlink-rp.x уже добавляет
    // сам embassy-rp в своём build.rs — дублировать их не нужно.
    // Добавляем только скрипт для defmt.
    println!("cargo:rustc-link-arg-bins=-Tdefmt.x");
}
