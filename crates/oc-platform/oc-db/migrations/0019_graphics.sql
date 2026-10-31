-- Graphics & icon library (A4): keep the sanitised SVG source of `svg` assets so they can be recoloured
-- non-destructively (the renderable raster stays at sha256; recoloured variants are derived from this source).
ALTER TABLE assets ADD COLUMN IF NOT EXISTS svg_sha256 text;
