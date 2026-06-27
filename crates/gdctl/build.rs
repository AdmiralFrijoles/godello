//! Build script.
//!
//! The only job here is to rebuild when GODELLO_VERSION changes, so a release
//! build always embeds the version it was given even if an earlier build is
//! cached. The version itself is read with option_env in the crate.

fn main() {
    println!("cargo:rerun-if-env-changed=GODELLO_VERSION");
}
