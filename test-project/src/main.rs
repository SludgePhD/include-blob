fn main() {
    let data = include_blob::include_blob!("blobs/file.txt");
    let s = std::str::from_utf8(data).unwrap();
    println!("{s}");
}
