-- A brand kit's motion rules: how the brand's videos and animated designs move (entrance type, durations, easing,
-- travel, stagger, page transitions, and whether anything may overshoot). The AI connector uses them as the defaults
-- of its animation operations, so every brand's videos move in its own style.
ALTER TABLE brand_kits ADD COLUMN motion jsonb NOT NULL DEFAULT '{}';
-- { "transition": { "type": "fade", "duration": 150 }, "draw": { "duration": 400 },
--   "text": { "type": "rise", "distance": 8, "duration": 300, "easing": "ease-out" }, "stagger": 60, "bounce": false }
