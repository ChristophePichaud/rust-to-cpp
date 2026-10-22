use std::fs;
use std::path::Path;

use rust_to_cpp::{Options, transpile_str};

#[test]
fn feature_fixtures_match_generated_cpp() {
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let mut inputs: Vec<_> = fs::read_dir(&fixtures)
        .expect("read fixture directory")
        .map(|entry| entry.expect("read fixture entry").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "rs"))
        .collect();
    inputs.sort();

    assert!(!inputs.is_empty(), "expected at least one feature fixture");

    for input in inputs {
        let stem = input.file_stem().unwrap().to_string_lossy();
        let source = fs::read_to_string(&input).expect("read Rust fixture");
        let options = Options {
            namespace: vec![stem.to_string()],
            ..Options::default()
        };
        let output = transpile_str(&source, &stem, &options)
            .unwrap_or_else(|error| panic!("parse {}: {error}", input.display()));
        let expected_header = fs::read_to_string(input.with_extension("h"))
            .unwrap_or_else(|error| panic!("read {}.h: {error}", input.display()));
        let expected_source = fs::read_to_string(input.with_extension("cpp"))
            .unwrap_or_else(|error| panic!("read {}.cpp: {error}", input.display()));

        assert_eq!(output.header, expected_header, "{} header changed", stem);
        assert_eq!(output.source, expected_source, "{} source changed", stem);
    }
}
