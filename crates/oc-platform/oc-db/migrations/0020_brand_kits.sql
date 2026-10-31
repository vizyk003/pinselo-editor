-- Brand Hub (A8), part 1: many brand kits per workspace (team or personal) with a default kit.
-- The v1 table had one row per workspace keyed by workspace_id; existing rows become each workspace's default kit.
ALTER TABLE brand_kits ADD COLUMN id uuid;
UPDATE brand_kits SET id = gen_random_uuid();
ALTER TABLE brand_kits ALTER COLUMN id SET NOT NULL;
ALTER TABLE brand_kits DROP CONSTRAINT brand_kits_pkey;
ALTER TABLE brand_kits ADD PRIMARY KEY (id);
ALTER TABLE brand_kits ALTER COLUMN workspace_id SET NOT NULL;
ALTER TABLE brand_kits ADD COLUMN is_default boolean NOT NULL DEFAULT false;
UPDATE brand_kits SET is_default = true;
ALTER TABLE brand_kits ADD COLUMN palettes jsonb NOT NULL DEFAULT '[]';  -- [{ "name": "Spring", "colors": [{ "name": "Mint", "hex": "#a7f3d0" }] }]
ALTER TABLE brand_kits ADD COLUMN photos jsonb NOT NULL DEFAULT '[]';    -- [asset id]
ALTER TABLE brand_kits ADD COLUMN graphics jsonb NOT NULL DEFAULT '[]';  -- [asset id]
ALTER TABLE brand_kits ADD COLUMN voice text NOT NULL DEFAULT '';        -- brand voice / tone notes
ALTER TABLE brand_kits ADD COLUMN created_by uuid REFERENCES users(id) ON DELETE SET NULL;
ALTER TABLE brand_kits ADD COLUMN created_at timestamptz NOT NULL DEFAULT now();
-- fonts gains "quote" and "caption" roles (jsonb, no change needed).
CREATE UNIQUE INDEX brand_kits_one_default ON brand_kits(workspace_id) WHERE is_default;
CREATE INDEX brand_kits_ws ON brand_kits(workspace_id, created_at);
