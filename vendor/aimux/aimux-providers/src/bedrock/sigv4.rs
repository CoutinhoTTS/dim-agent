//! AWS Signature Version 4 signing backed by the official AWS implementation.

use std::time::SystemTime;

use aws_credential_types::Credentials;
use aws_sigv4::http_request::{SignableBody, SignableRequest, SigningSettings, sign};
use aws_sigv4::sign::v4;

use aimux_core::error::AiMuxError;

#[derive(Debug, Clone)]
pub struct AwsCredentials {
    pub access_key_id: String,
    pub secret_access_key: String,
    pub session_token: Option<String>,
    pub region: String,
}

#[derive(Debug, Clone)]
pub struct SignedRequest {
    pub headers: Vec<(String, String)>,
}

pub fn sign_request(
    credentials: &AwsCredentials,
    service: &str,
    method: &str,
    url: &str,
    body: &str,
    extra_headers: &[(String, String)],
) -> Result<SignedRequest, AiMuxError> {
    sign_request_at(
        credentials,
        service,
        method,
        url,
        body.as_bytes(),
        extra_headers,
        SystemTime::now(),
    )
}

fn sign_request_at(
    credentials: &AwsCredentials,
    service: &str,
    method: &str,
    url: &str,
    body: &[u8],
    extra_headers: &[(String, String)],
    time: SystemTime,
) -> Result<SignedRequest, AiMuxError> {
    const RESERVED: &[&str] = &[
        "authorization",
        "host",
        "x-amz-date",
        "x-amz-content-sha256",
        "x-amz-security-token",
    ];
    if let Some((name, _)) = extra_headers.iter().find(|(name, _)| {
        RESERVED
            .iter()
            .any(|reserved| name.eq_ignore_ascii_case(reserved))
    }) {
        return Err(AiMuxError::InvalidArgument(format!(
            "header {name} is reserved for AWS request signing"
        )));
    }

    let identity = Credentials::new(
        &credentials.access_key_id,
        &credentials.secret_access_key,
        credentials.session_token.clone(),
        None,
        "explicit",
    )
    .into();
    let params: aws_sigv4::http_request::SigningParams<'_> = v4::SigningParams::builder()
        .identity(&identity)
        .region(&credentials.region)
        .name(service)
        .time(time)
        .settings(SigningSettings::default())
        .build()
        .map_err(|error| AiMuxError::Other(format!("invalid AWS signing parameters: {error}")))?
        .into();
    let request = SignableRequest::new(
        method,
        url,
        extra_headers
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str())),
        SignableBody::Bytes(body),
    )
    .map_err(|error| AiMuxError::Other(format!("invalid AWS signable request: {error}")))?;
    let (instructions, _) = sign(request, &params)
        .map_err(|error| AiMuxError::Other(format!("AWS request signing failed: {error}")))?
        .into_parts();
    let mut headers = extra_headers.to_vec();
    headers.extend(
        instructions
            .headers()
            .map(|(name, value)| (name.to_string(), value.to_string())),
    );
    Ok(SignedRequest { headers })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, UNIX_EPOCH};

    #[test]
    fn fixed_time_aws_fixture() {
        let signed = sign_request_at(
            &AwsCredentials {
                access_key_id: "AKIDEXAMPLE".into(),
                secret_access_key: "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY".into(),
                session_token: None,
                region: "us-east-1".into(),
            },
            "iam",
            "GET",
            "https://iam.amazonaws.com/?Action=ListUsers&Version=2010-05-08",
            b"",
            &[(
                "Content-Type".into(),
                "application/x-www-form-urlencoded; charset=utf-8".into(),
            )],
            UNIX_EPOCH + Duration::from_secs(1_440_938_160),
        )
        .unwrap();
        let authorization = signed
            .headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("authorization"))
            .unwrap()
            .1
            .as_str();
        assert_eq!(
            authorization,
            "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20150830/us-east-1/iam/aws4_request, SignedHeaders=content-type;host;x-amz-date, Signature=5d672d79c15b13162d9279b0855cfba6789a8edb4c82c400e06b5924a6f2b5d7"
        );
    }

    #[test]
    fn rejects_reserved_custom_header() {
        assert!(
            sign_request_at(
                &AwsCredentials {
                    access_key_id: "a".into(),
                    secret_access_key: "b".into(),
                    session_token: None,
                    region: "us-east-1".into()
                },
                "bedrock",
                "POST",
                "https://example.com/",
                b"{}",
                &[("HOST".into(), "evil".into())],
                SystemTime::now()
            )
            .is_err()
        );
    }
}
