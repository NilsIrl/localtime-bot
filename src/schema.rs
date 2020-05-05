table! {
    roles (id) {
        id -> Int8,
        guild_id -> Int8,
        refresh_interval -> Interval,
        timezone -> Text,
    }
}
