use super::*;

// Low-entropy test secrets, built at run time: a secret-shaped literal in a
// test would read as a leaked credential to the secret scan.
fn secret(byte: u8) -> LinkSecret {
    LinkSecret::from_bytes([byte; SECRET_BYTES])
}

fn line(bytes: &[u8]) -> String {
    format!("{}\n", STANDARD.encode(bytes))
}

#[test]
fn a_secret_is_32_bytes_of_base64() {
    let read = read_secret(line(&[7; 32]).as_bytes()).unwrap();
    assert_eq!(read, secret(7));
    let padded = format!("  {}\r\n", STANDARD.encode([9u8; 32]));
    assert_eq!(read_secret(padded.as_bytes()).unwrap(), secret(9));
    assert_eq!(STANDARD.encode([7u8; 32]).len(), 44);
    for bad in [
        line(&[7; 31]),
        line(&[7; 33]),
        "not base64 at all !!\n".to_string(),
        String::new(),
        "\n".to_string(),
    ] {
        let error = read_secret(bad.as_bytes()).unwrap_err();
        assert!(matches!(error, ProvisionError::Invalid(_)), "{bad:?}");
        assert_eq!(error.to_string(), "the link secret is 32 bytes in base64");
    }
}

#[test]
fn the_secret_survives_store_and_load() {
    let dir = tempfile::tempdir().unwrap();
    let secrets = dir.path().join(SECRETS_DIR);
    assert_eq!(load(&secrets).unwrap(), None, "none set yet");
    run(dir.path(), line(&[3; 32]).as_bytes()).unwrap();
    assert_eq!(load(&secrets).unwrap(), Some(secret(3)));
    run(dir.path(), line(&[4; 32]).as_bytes()).unwrap();
    assert_eq!(load(&secrets).unwrap(), Some(secret(4)), "replaced");
    assert_eq!(
        secret_path(&secrets),
        secrets.join(format!("link_secret.{}", crate::sealed::SEALED_EXT))
    );
    assert!(secret_path(&secrets).is_file());
}

#[test]
fn a_bad_secret_stores_nothing_and_a_bad_file_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    assert!(run(dir.path(), "x\n".as_bytes()).is_err());
    assert!(!dir.path().join(SECRETS_DIR).exists());
    let secrets = dir.path().join(SECRETS_DIR);
    std::fs::create_dir_all(&secrets).unwrap();
    crate::sealed::write(&secret_path(&secrets), &[1; 31]).unwrap();
    let error = load(&secrets).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    assert!(error.to_string().contains("link set-secret"), "{error}");
}

#[test]
fn the_secret_never_shows_in_debug() {
    assert_eq!(format!("{:?}", secret(5)), "LinkSecret(..)");
}

fn bearer(token: &str) -> String {
    format!("Bearer {token}")
}

#[test]
fn a_fresh_hub_token_is_accepted() {
    let token = link_token(&secret(1), 1_000);
    assert_eq!(
        link_claims_ok(Some(&bearer(&token)), &secret(1), 1_000),
        Ok(())
    );
    let claims = decode::<LinkClaims>(&token, &DecodingKey::from_secret(&[1; SECRET_BYTES]), &{
        let mut v = Validation::new(Algorithm::HS256);
        v.validate_exp = false;
        v
    })
    .unwrap()
    .claims;
    assert_eq!(
        claims,
        LinkClaims {
            sub: "hub".to_string(),
            iat: 1_000,
            exp: 1_060,
        }
    );
}

#[test]
fn a_token_of_another_secret_is_refused() {
    let token = link_token(&secret(2), 1_000);
    assert_eq!(
        link_claims_ok(Some(&bearer(&token)), &secret(1), 1_000),
        Err(LinkRefusal::BadToken)
    );
    assert_eq!(
        link_claims_ok(Some("Bearer not.a.jwt"), &secret(1), 1_000),
        Err(LinkRefusal::BadToken)
    );
}

#[test]
fn a_token_61_s_old_is_expired_and_60_s_is_not() {
    let token = link_token(&secret(1), 1_000);
    let header = bearer(&token);
    assert_eq!(link_claims_ok(Some(&header), &secret(1), 1_060), Ok(()));
    assert_eq!(
        link_claims_ok(Some(&header), &secret(1), 1_061),
        Err(LinkRefusal::Expired)
    );
}

#[test]
fn a_token_for_another_subject_is_refused() {
    let claims = LinkClaims {
        sub: "engineer".to_string(),
        iat: 1_000,
        exp: 1_060,
    };
    let token = encode(
        &Header::new(Algorithm::HS256),
        &claims,
        &EncodingKey::from_secret(&[1; SECRET_BYTES]),
    )
    .unwrap();
    assert_eq!(
        link_claims_ok(Some(&bearer(&token)), &secret(1), 1_000),
        Err(LinkRefusal::NotHub)
    );
}

#[test]
fn no_bearer_header_is_refused() {
    let token = link_token(&secret(1), 1_000);
    assert_eq!(
        link_claims_ok(None, &secret(1), 1_000),
        Err(LinkRefusal::NoHeader)
    );
    assert_eq!(
        link_claims_ok(Some(&token), &secret(1), 1_000),
        Err(LinkRefusal::NoHeader),
        "a token without `Bearer `"
    );
}

#[test]
fn every_refusal_has_its_words() {
    assert_eq!(LinkRefusal::NoHeader.text(), "no bearer token");
    assert_eq!(
        LinkRefusal::BadToken.text(),
        "not a token of the link secret"
    );
    assert_eq!(LinkRefusal::Expired.text(), "the token expired");
    assert_eq!(LinkRefusal::NotHub.text(), "the token is not a hub's");
    assert_eq!(LinkRefusal::NoSecret.text(), "no link secret");
}

#[test]
fn a_secret_that_cannot_be_read_is_none_with_a_warning() {
    assert_eq!(usable(Ok(Some(secret(1)))), (Some(secret(1)), None));
    assert_eq!(usable(Ok(None)), (None, None));
    let (secret, warning) = usable(Err(std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        "not 32 bytes",
    )));
    assert_eq!(secret, None);
    assert_eq!(
        warning.as_deref(),
        Some("the link secret cannot be read, the instance link runs without it: not 32 bytes")
    );
}
