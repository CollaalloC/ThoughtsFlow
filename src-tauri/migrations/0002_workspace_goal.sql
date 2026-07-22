ALTER TABLE workspace ADD COLUMN goal TEXT NOT NULL DEFAULT '';

-- v1 exposed `goal` in the UI but persisted it in `system_prompt`. Preserve a
-- real goal while preventing the old placeholder or goal text from being sent
-- to providers as a system instruction after upgrade.
UPDATE workspace
SET goal = CASE
        WHEN system_prompt = '尚未设置工作区目标' THEN ''
        ELSE system_prompt
    END,
    system_prompt = 'You are a careful technical reasoning partner. Make assumptions explicit and preserve competing options.'
WHERE system_prompt <> 'You are a careful technical reasoning partner. Make assumptions explicit and preserve competing options.';
