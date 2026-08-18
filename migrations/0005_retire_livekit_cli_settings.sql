-- Retire the settings that described the `lk` child-process publisher.
--
-- ADR 011 replaced that publisher with in-process encoded publication, so
-- `camera.livekit.cli_path`, `camera.livekit.cli_version` and
-- `camera.livekit.ipc_write_timeout` no longer name anything the process can
-- do. They are gone from the classification registry, and the settings cache
-- treats a persisted key it cannot classify as corruption rather than as noise
-- to ignore. Leaving the rows in place would therefore stop an upgraded
-- deployment from loading its configuration at all.
--
-- ADR 008 requires that a settings change and its audit record commit in one
-- transaction. This is a settings change: it is recorded, with the same
-- generation for every row, before the rows are deleted. Migrations run inside
-- a transaction, so the audit and the deletion commit together or not at all.

DO $$
DECLARE
    retired_generation bigint;
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM settings
        WHERE setting_key IN (
            'camera.livekit.cli_path',
            'camera.livekit.cli_version',
            'camera.livekit.ipc_write_timeout'
        )
    ) THEN
        RETURN;
    END IF;

    retired_generation := nextval('settings_generation_sequence');

    INSERT INTO settings_audit (
        generation, actor, action, setting_key, camera_id, origin, outcome,
        old_value, new_value
    )
    SELECT
        retired_generation,
        'migration:0005_retire_livekit_cli_settings',
        'clear',
        setting_key,
        camera_id,
        'migration',
        'applied',
        -- None of the three is a secret-classified key, so the plaintext
        -- previous value is the honest old side. `settings_no_secret_keys`
        -- already guarantees no credential ever reached this table.
        value,
        NULL
    FROM settings
    WHERE setting_key IN (
        'camera.livekit.cli_path',
        'camera.livekit.cli_version',
        'camera.livekit.ipc_write_timeout'
    );

    DELETE FROM settings
    WHERE setting_key IN (
        'camera.livekit.cli_path',
        'camera.livekit.cli_version',
        'camera.livekit.ipc_write_timeout'
    );
END
$$;
