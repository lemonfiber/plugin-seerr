use serde::Deserialize;

use lemonfiber_ports::media::Kind;
use lemonfiber_ports::service::{HouseholdRequest, MediaStatus, RequestStatus};

pub(super) const REQUEST_PAGE: usize = 100;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct RequestPage {
    #[serde(default)]
    pub(super) page_info: PageInfo,
    #[serde(default)]
    pub(super) results: Vec<RequestRecord>,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PageInfo {
    #[serde(default)]
    pub(super) results: usize,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct RequestRecord {
    #[serde(default)]
    pub(super) id: i64,
    #[serde(default)]
    pub(super) created_at: Option<String>,
    #[serde(default)]
    pub(super) status: u8,
    #[serde(default, rename = "type")]
    pub(super) media_type: String,
    #[serde(default)]
    pub(super) media: MediaRecord,
    #[serde(default)]
    pub(super) requested_by: MemberRecord,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct MediaRecord {
    #[serde(default)]
    pub(super) status: u8,
    #[serde(default)]
    pub(super) tmdb_id: Option<i64>,
    #[serde(default)]
    pub(super) external_service_id: Option<i64>,
    #[serde(default)]
    pub(super) media_added_at: Option<String>,
    #[serde(default)]
    pub(super) jellyfin_media_id: Option<String>,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct MemberRecord {
    #[serde(default)]
    pub(super) display_name: String,
    #[serde(default)]
    pub(super) jellyfin_user_id: Option<String>,
}

impl RequestRecord {
    pub(super) fn into_request(self) -> HouseholdRequest {
        HouseholdRequest {
            id: self.id,
            made: self.created_at,
            member: self.requested_by.display_name,
            member_id: self.requested_by.jellyfin_user_id,
            kind: kind_of(&self.media_type),
            title_id: self.media.tmdb_id.map(|id| id.to_string()),
            item: self.media.external_service_id,
            arrived: self.media.media_added_at,
            shelf_id: self.media.jellyfin_media_id,
            request_status: request_status(self.status),
            media_status: media_status(self.media.status),
        }
    }
}

pub(super) const fn media_type(kind: Kind) -> &'static str {
    match kind {
        Kind::Tv => "tv",
        Kind::Movies => "movie",
    }
}

pub(super) fn kind_of(word: &str) -> Option<Kind> {
    Kind::ALL.into_iter().find(|kind| media_type(*kind) == word)
}

pub(super) const fn request_status(number: u8) -> Option<RequestStatus> {
    match number {
        1 => Some(RequestStatus::Pending),
        2 => Some(RequestStatus::Approved),
        3 => Some(RequestStatus::Declined),
        4 => Some(RequestStatus::Failed),
        5 => Some(RequestStatus::Completed),
        _ => None,
    }
}

pub(super) const fn media_status(number: u8) -> Option<MediaStatus> {
    match number {
        1 => Some(MediaStatus::Unknown),
        2 => Some(MediaStatus::Pending),
        3 => Some(MediaStatus::Processing),
        4 => Some(MediaStatus::PartlyAvailable),
        5 => Some(MediaStatus::Available),
        7 => Some(MediaStatus::Deleted),
        _ => None,
    }
}

#[cfg(test)]
mod tests;
