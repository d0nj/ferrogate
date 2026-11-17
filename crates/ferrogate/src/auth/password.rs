use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};

pub const MIN_PASSWORD_LEN: usize = 12;

const BOOTSTRAP_PASSWORD: &str = "ferrogate";

pub fn hash_password(password: &str) -> Result<String, argon2::password_hash::Error> {
    let salt = SaltString::generate(&mut OsRng);
    Ok(Argon2::default()
        .hash_password(password.as_bytes(), &salt)?
        .to_string())
}

pub fn verify_password(password: &str, phc: &str) -> bool {
    let Ok(parsed) = PasswordHash::new(phc) else {
        return false;
    };
    Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok()
}

pub fn verify_dummy(password: &str) {
    const DUMMY_PHC: &str = "$argon2id$v=19$m=19456,t=2,p=1$bftJwxMRawCtw3G1GuQ44Q$E2iMW0Npj4FG1MP9VJ1dU8eVMa891oVRhV/oBN+a428";
    let parsed = PasswordHash::new(DUMMY_PHC).expect("dummy PHC is well formed");
    let _ = Argon2::default().verify_password(password.as_bytes(), &parsed);
}

pub fn validate_password(password: &str) -> Result<(), &'static str> {
    if password.chars().count() < MIN_PASSWORD_LEN {
        return Err("Password must be at least 12 characters.");
    }
    if password == BOOTSTRAP_PASSWORD {
        return Err("Password must not be the default bootstrap password.");
    }
    if is_flat_lowercase(password) {
        return Err("Password must contain a digit or symbol.");
    }
    Ok(())
}

fn is_flat_lowercase(password: &str) -> bool {
    password == password.to_lowercase()
        && !password
            .chars()
            .any(|c| c.is_ascii_digit() || !c.is_alphanumeric())
}

pub async fn verify_password_offloaded(password: String, phc: String) -> bool {
    tokio::task::spawn_blocking(move || verify_password(&password, &phc))
        .await
        .unwrap_or(false)
}

pub async fn verify_dummy_offloaded(password: String) {
    let _ = tokio::task::spawn_blocking(move || verify_dummy(&password)).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_round_trips() {
        let phc = hash_password("correct horse battery 9").expect("hash");
        assert!(verify_password("correct horse battery 9", &phc));
        assert!(!verify_password("correct horse battery 8", &phc));
    }

    #[test]
    fn hashes_are_salted() {
        let a = hash_password("correct horse battery 9").expect("hash");
        let b = hash_password("correct horse battery 9").expect("hash");
        assert_ne!(a, b);
    }

    #[test]
    fn garbage_phc_is_rejected_without_panicking() {
        for phc in ["", "not-a-hash", "$argon2id$v=19$m=x,t=y", "$argon2id$"] {
            assert!(!verify_password("anything", phc));
        }
    }

    #[test]
    fn validate_rejects_bootstrap_default() {
        assert!(validate_password("ferrogate").is_err());
    }

    #[test]
    fn validate_rejects_short_passwords() {
        assert!(validate_password("Sh0rt!").is_err());
        assert!(validate_password("").is_err());
    }

    #[test]
    fn validate_rejects_flat_lowercase() {
        assert!(validate_password("plainlowercasepassword").is_err());
    }

    #[test]
    fn validate_accepts_strong_passwords() {
        assert!(validate_password("Correct-Horse-9").is_ok());
        assert!(validate_password("alllowercase9").is_ok());
        assert!(validate_password("with symbol here").is_ok());
    }
}
