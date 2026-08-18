//! Mints one bearer token and prints it with its Argon2id hash.
//!
//! This exists because there is otherwise no way to create the first identity:
//! with a database configured and no principals, the API refuses everything and
//! a routable listener will not start. The alternatives — a loopback backdoor
//! or a default credential — are both worse than a deliberate manual step.
//!
//! An **example**, not a shipped binary. It is built and linted by
//! `--all-targets`, so it cannot rot, but it is not a product surface. Promoting
//! it to a real subcommand is a decision that belongs with the packaging work.
//!
//! ```text
//! cargo run -p surveillance-auth --example mint-token -- <principal-id> <role>
//! ```
//!
//! The token is printed **once** and is not recoverable: only its Argon2id hash
//! is meant to be stored. Treat the output as a credential — it will be in your
//! shell history and scrollback.

use std::process::ExitCode;

use surveillance_auth::{
    clock::{Clock, SystemClock},
    principal::Role,
    token::IssuedToken,
    verify::{HashParams, Verifier},
};

/// Default lifetime. Expiry is mandatory, so there is no "never" to choose.
const DEFAULT_LIFETIME_SECONDS: i64 = 90 * 24 * 60 * 60;

fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let (Some(principal), Some(role)) = (arguments.first(), arguments.get(1)) else {
        eprintln!(
            "usage: mint-token <principal-id> <viewer|operator|administrator> [lifetime-days]"
        );
        return ExitCode::FAILURE;
    };

    let Ok(role) = Role::parse(role) else {
        eprintln!("unrecognised role '{role}': expected viewer, operator, or administrator");
        return ExitCode::FAILURE;
    };

    let lifetime = match arguments.get(2) {
        Some(days) => match days.parse::<i64>() {
            Ok(days) if days > 0 => days.saturating_mul(24 * 60 * 60),
            _ => {
                eprintln!("lifetime-days must be a positive whole number");
                return ExitCode::FAILURE;
            }
        },
        None => DEFAULT_LIFETIME_SECONDS,
    };

    let expires_at = SystemClock.now().saturating_add_seconds(lifetime);
    let Ok(token) = IssuedToken::generate(expires_at) else {
        eprintln!("the operating system entropy source is unavailable; no token was minted");
        return ExitCode::FAILURE;
    };

    // Production parameters, so the row is verifiable by the running server.
    let Ok(verifier) = Verifier::new(HashParams::default()) else {
        eprintln!("could not build the verifier");
        return ExitCode::FAILURE;
    };
    let Ok(hash) = verifier.hash(token.secret()) else {
        eprintln!("hashing failed; no token was minted");
        return ExitCode::FAILURE;
    };

    println!("token (shown once, not recoverable):");
    println!("  {}", token.render().expose());
    println!();
    println!("SQL to store it — the plaintext above is never stored:");
    println!(
        "  INSERT INTO principals (principal_id, kind, display_name, role, scope_all, enabled)"
    );
    println!(
        "  VALUES ('{}', 'human', '{}', '{}', true, true);",
        principal,
        principal,
        role.as_str()
    );
    println!("  INSERT INTO auth_tokens (token_id, principal_id, credential_hash, expires_at)");
    println!(
        "  VALUES ('{}', '{}', '{}', to_timestamp({}));",
        token.id(),
        principal,
        hash.as_str(),
        expires_at.as_unix_seconds()
    );

    ExitCode::SUCCESS
}
