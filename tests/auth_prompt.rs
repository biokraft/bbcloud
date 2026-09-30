//! Drives `bb auth login` through a real pseudo-terminal, because the prompts
//! refuse to run on the piped stdin every other integration test uses. rexpect
//! decodes output byte by byte, so expectations stay ASCII.
#![cfg(unix)]
#![allow(clippy::unwrap_used)]

use rexpect::session::{spawn_command, PtySession};

fn login(home: &std::path::Path) -> PtySession {
    let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_bb"));
    cmd.args(["auth", "login"])
        .env("HOME", home)
        .env("XDG_CONFIG_HOME", home)
        .env("NO_COLOR", "1")
        .env("BB_NO_UPDATE_CHECK", "1")
        .env("BB_KEYRING_DISABLE", "1")
        .env_remove("BB_EMAIL")
        .env_remove("BB_TOKEN");
    spawn_command(cmd, Some(10_000)).unwrap()
}

#[test]
fn ctrl_c_at_the_email_prompt_names_that_prompt() {
    let home = tempfile::tempdir().unwrap();
    let mut bb = login(home.path());

    bb.exp_string("email:").unwrap();
    bb.send_control('c').unwrap();
    bb.exp_string("login cancelled at the email prompt")
        .unwrap();
    bb.exp_string("no token was checked").unwrap();
}

#[test]
fn ctrl_c_at_the_token_prompt_does_not_read_as_a_rejected_token() {
    let home = tempfile::tempdir().unwrap();
    let mut bb = login(home.path());

    bb.exp_string("email:").unwrap();
    bb.send("dev@example.com\r").unwrap();
    bb.flush().unwrap();
    bb.exp_string("api token:").unwrap();
    bb.send_control('c').unwrap();
    bb.exp_string("login cancelled at the api token prompt")
        .unwrap();
    bb.exp_string("no token was checked").unwrap();
}
