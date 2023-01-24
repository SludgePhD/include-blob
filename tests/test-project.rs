use std::process::Command;

#[test]
fn run_test_project() {
    let cargo = std::env::var("CARGO").unwrap();
    println!("CARGO={cargo}");
    let output = Command::new(cargo)
        .arg("run")
        .current_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/test-project"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}, {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("non UTF-8 output");
    assert!(stdout.contains("Contents of file.txt"), "{stdout}");
}
