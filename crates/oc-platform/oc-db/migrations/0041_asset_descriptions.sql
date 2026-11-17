-- What an upload shows and what it is good for, so an AI assistant can pick the right one without opening it: a
-- plain description and search tags (lower case). Written with the AI connector's describe_asset; shown by
-- get_brand_kit and list_uploads, and list_uploads searches them.
ALTER TABLE assets ADD COLUMN description text NOT NULL DEFAULT '';
ALTER TABLE assets ADD COLUMN tags text[] NOT NULL DEFAULT '{}';
