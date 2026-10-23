use serde::Deserialize;

pub(super) const LINK_MEMBERS: &str = "/user/import-from-jellyfin";

pub(super) const NOT_FOUND: u16 = 404;

pub(super) const MEMBERS: &str = "/user";

pub(super) const PERMISSIONS: &str = "settings/permissions";

const APPROVES_OWN: u64 = 2 | 128 | 256 | 512 | 32_768 | 65_536 | 131_072;

const GRANTS_APPROVAL: u64 = 128;

const ASKS_AT_ALL: u64 = 32 | 1_024 | 2_048 | 4_096 | 262_144 | 524_288;

const ADMINISTERS: u64 = 2;

#[derive(Deserialize)]
pub(super) struct MemberResource {
    #[serde(default)]
    pub(super) id: i64,
    #[serde(default)]
    pub(super) permissions: u64,
}

#[derive(Deserialize)]
pub(super) struct PermissionsResource {
    #[serde(default)]
    pub(super) permissions: u64,
}

#[must_use]
pub(super) const fn approves_own(permissions: u64) -> bool {
    permissions & APPROVES_OWN != 0
}

#[must_use]
pub(super) const fn without_approval(permissions: u64) -> u64 {
    permissions & !APPROVES_OWN
}

#[must_use]
pub(super) const fn with_approval(permissions: u64) -> u64 {
    permissions | GRANTS_APPROVAL
}

#[must_use]
pub(super) const fn without_asking(permissions: u64) -> (u64, u64) {
    if permissions & ADMINISTERS != 0 {
        return (permissions, 0);
    }
    (permissions & !ASKS_AT_ALL, permissions & ASKS_AT_ALL)
}

#[must_use]
pub(super) const fn with_asking(permissions: u64, taken: u64) -> u64 {
    permissions | taken
}

#[cfg(test)]
mod tests;
