CREATE TABLE roles (
	id BIGINT PRIMARY KEY NOT NULL,
	guild_id BIGINT NOT NULL,
	refresh_interval INTERVAL DEFAULT '1 minute' NOT NULL,
	timezone TEXT NOT NULL CHECK (now() AT TIME ZONE timezone IS NOT NULL)
)
