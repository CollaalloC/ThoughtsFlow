-- SQLx applies this entire migration in one transaction, including its history
-- row. Only mutable Provider Profiles are renamed; historical snapshots,
-- requests, Agent receipts, and their hashes are never rewritten.
-- An already-present new key is accepted only when both type and value match.
-- Conflicting or duplicate markers require explicit repair, never a silent
-- change of the user's default selection.
CREATE TEMP TABLE provider_default_marker_guard (
    valid INTEGER CONSTRAINT conflicting_provider_default_markers CHECK (valid = 1)
);

INSERT INTO provider_default_marker_guard (valid)
SELECT 0
WHERE EXISTS (
    SELECT 1 FROM provider_profile
    WHERE json_type(parameters_json, '$._thoughsflowIsDefault') IS NOT NULL
      AND json_type(parameters_json, '$._thoughtsflowIsDefault') IS NOT NULL
      AND (
          json_type(parameters_json, '$._thoughsflowIsDefault')
              IS NOT json_type(parameters_json, '$._thoughtsflowIsDefault')
          OR json_extract(parameters_json, '$._thoughsflowIsDefault')
              IS NOT json_extract(parameters_json, '$._thoughtsflowIsDefault')
      )
)
OR EXISTS (
    SELECT 1 FROM provider_profile, json_each(provider_profile.parameters_json)
    WHERE json_each.key IN ('_thoughsflowIsDefault', '_thoughtsflowIsDefault')
    GROUP BY provider_profile.id, json_each.key
    HAVING COUNT(*) > 1
);

-- json_extract returns booleans as SQL integers, so preserve their JSON types
-- explicitly. json_quote retains every string value (including whitespace and
-- escapes); json(...) also preserves numbers, null, arrays, and objects.
-- These JSON1 functions are supported by the existing SQLite 3.37 minimum.
UPDATE provider_profile
SET parameters_json = json_remove(
    json_set(
        parameters_json,
        '$._thoughtsflowIsDefault',
        json(CASE json_type(parameters_json, '$._thoughsflowIsDefault')
            WHEN 'true' THEN 'true'
            WHEN 'false' THEN 'false'
            ELSE json_quote(json_extract(parameters_json, '$._thoughsflowIsDefault'))
        END)
    ),
    '$._thoughsflowIsDefault'
)
WHERE json_type(parameters_json, '$._thoughsflowIsDefault') IS NOT NULL;

DROP TABLE provider_default_marker_guard;
