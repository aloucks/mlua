#[cfg(any(feature = "lua53", feature = "lua51"))]
#[test]
fn test_compile_fail() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/compile_fail/*.rs");
}
