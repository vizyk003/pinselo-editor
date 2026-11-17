-- A brand kit's videos: clips everyone who can see the kit drops into designs (the editor's Brand panel, the AI
-- connector's add_video { asset }). Asset ids of video uploads, like logos, photos and graphics.
ALTER TABLE brand_kits ADD COLUMN videos jsonb NOT NULL DEFAULT '[]';
