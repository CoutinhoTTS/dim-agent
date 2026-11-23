//! The destination format rule a request, a standalone call and the provider
//! encoder share: each protocol takes only the image formats its documents
//! list, and a destination whose protocol is unknown takes the four a request
//! may carry.
use dim_agent_capabilities::images::takes_image_format;
use dim_agent_types::ModelProtocol;

const REQUEST_FORMATS: [&str; 4] = ["image/png", "image/jpeg", "image/webp", "image/gif"];

/// One destination: the formats it takes and the ones it must refuse.
struct Destination {
    protocol: Option<ModelProtocol>,
    takes: &'static [&'static str],
    refuses: &'static [&'static str],
}

#[test]
fn a_destination_takes_only_the_image_formats_its_protocol_documents() {
    let four = &REQUEST_FORMATS[..];
    let destinations = [
        Destination {
            protocol: None,
            takes: four,
            refuses: &["image/heic", "image/svg+xml"],
        },
        Destination {
            protocol: Some(ModelProtocol::Anthropic),
            takes: four,
            refuses: &["image/heic", "image/svg+xml"],
        },
        Destination {
            protocol: Some(ModelProtocol::OpenaiChat),
            takes: four,
            refuses: &["image/heic", "image/svg+xml"],
        },
        Destination {
            protocol: Some(ModelProtocol::OpenaiResponses),
            takes: four,
            refuses: &["image/heic", "image/svg+xml"],
        },
        Destination {
            protocol: Some(ModelProtocol::OpenaiCompatible),
            takes: four,
            refuses: &["image/heic", "image/svg+xml"],
        },
        Destination {
            protocol: Some(ModelProtocol::Bedrock),
            takes: four,
            refuses: &["image/heic", "image/svg+xml"],
        },
        Destination {
            protocol: Some(ModelProtocol::Gemini),
            takes: &[
                "image/png",
                "image/jpeg",
                "image/webp",
                "image/heic",
                "image/heif",
            ],
            refuses: &["image/gif", "image/svg+xml"],
        },
        Destination {
            protocol: Some(ModelProtocol::Xai),
            takes: &["image/png", "image/jpeg"],
            refuses: &["image/webp", "image/gif", "image/heic"],
        },
        Destination {
            protocol: Some(ModelProtocol::XaiResponses),
            takes: &["image/png", "image/jpeg"],
            refuses: &["image/webp", "image/gif", "image/heic"],
        },
    ];
    for destination in destinations {
        for media_type in destination.takes {
            assert!(
                takes_image_format(destination.protocol, media_type),
                "{:?} takes {media_type}",
                destination.protocol
            );
        }
        for media_type in destination.refuses {
            assert!(
                !takes_image_format(destination.protocol, media_type),
                "{:?} refuses {media_type}",
                destination.protocol
            );
        }
    }
}
