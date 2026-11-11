-- A brand kit's text styles: how each kind of text looks beyond its font (weight, italic, capitals, letter spacing,
-- line height, a reference size and the colours it may take). The editor adds text in these styles, and the AI
-- connector hands them to assistants and checks their edits against them.
ALTER TABLE brand_kits ADD COLUMN text_styles jsonb NOT NULL DEFAULT '[]';
-- [{ "name": "Eyebrow", "role": "caption", "weight": 500, "uppercase": true, "letterSpacing": 140, "size": 13, "colors": ["#6b6b66"] }]
