-- A brand kit's written guidelines (its brand book or design system documentation), in sections that the editor
-- edits and the AI connector hands to assistants on request. Kept out of the kit lists, which load on every open.
ALTER TABLE brand_kits ADD COLUMN guidelines jsonb NOT NULL DEFAULT '[]';  -- [{ "title": "Colour", "body": "Markdown" }]
