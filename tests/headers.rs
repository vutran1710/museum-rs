//! `museum::headers::bearer`: a token becomes the `Authorization` header every HTTP store sends.

#![allow(clippy::unwrap_used)]

use museum::headers::AUTHORIZATION;
use museum::headers::bearer;
use rstest::rstest;

#[rstest]
#[case::token_becomes_authorization_header("t0ken", Ok("Bearer t0ken"))]
#[case::token_with_newline_refused("t0ken\nX-Injected: 1", Err(()))]
fn bearer_header_is_built(#[case] token: &str, #[case] expected: Result<&str, ()>) {
    let built = bearer(token)
        .map(|headers| headers[AUTHORIZATION].to_str().unwrap().to_owned())
        .map_err(|_| ());
    assert_eq!(built, expected.map(str::to_owned));
}
