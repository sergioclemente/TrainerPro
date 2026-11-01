-- Queue vault cleanup before removing the only durable connection identities.
INSERT INTO settings(key, value)
SELECT 'legacy_intervals_icu_credential:' || id, 'intervals_icu:' || id
FROM provider_connections WHERE provider = 'intervals_icu';

-- Include orphaned/retired cache entries, but protect definitions used by a
-- surviving local or provider schedule. Independent local copies have no origin.
CREATE TEMP TABLE removed_intervals_definitions AS
SELECT wd.id FROM workout_definitions wd
WHERE (
  wd.origin = 'intervals_icu' OR EXISTS (
    SELECT 1 FROM scheduled_workouts sw
    JOIN provider_connections pc ON pc.id = sw.provider_connection_id
    WHERE sw.workout_definition_id = wd.id AND pc.provider = 'intervals_icu'
  )
) AND NOT EXISTS (
  SELECT 1 FROM scheduled_workouts sw
  LEFT JOIN provider_connections pc ON pc.id = sw.provider_connection_id
  WHERE sw.workout_definition_id = wd.id
    AND (pc.provider IS NULL OR pc.provider != 'intervals_icu')
);

-- The migration runner disables foreign keys; ON DELETE SET NULL will not run.
UPDATE activities SET scheduled_workout_id = NULL
WHERE scheduled_workout_id IN (
  SELECT sw.id FROM scheduled_workouts sw
  JOIN provider_connections pc ON pc.id = sw.provider_connection_id
  WHERE pc.provider = 'intervals_icu'
);
UPDATE activities SET workout_definition_id = NULL
WHERE workout_definition_id IN (SELECT id FROM removed_intervals_definitions);

DELETE FROM activity_uploads WHERE provider_connection_id IN (
  SELECT id FROM provider_connections WHERE provider = 'intervals_icu'
);
DELETE FROM scheduled_workouts WHERE provider_connection_id IN (
  SELECT id FROM provider_connections WHERE provider = 'intervals_icu'
);
DELETE FROM workout_definitions WHERE id IN (SELECT id FROM removed_intervals_definitions);
DROP TABLE removed_intervals_definitions;
DELETE FROM provider_connections WHERE provider = 'intervals_icu';

ALTER TABLE activities DROP COLUMN icu_activity_id;
