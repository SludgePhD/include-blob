# `include-blob`

`include-blob` is a small crate that provides a replacement for
[`include_bytes!`] that does not have the same severe impact on compile times
when used with large files (several MB).

It works by pre-processing the files to be included in a build script, bundling
them into static libraries, and telling Cargo to link against them.

[`include_bytes!`]: https://doc.rust-lang.org/stable/std/macro.include_bytes.html
