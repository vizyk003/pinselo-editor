//! Port of services/api/src/mcp/guide.ts — what assistants are told about designing in Pinselo: short server
//! instructions plus the full guide tool.

const INSTRUCTIONS: &str = r#"{brand} is a design editor (graphics, presentations, documents, websites, video). These tools create and edit the user's designs. Before designing for a brand, call get_brand_kit and use the kit whose name matches the design system or brand the user selected or named (create the design with create_design { brand_kit }): place its logo, and use its graphics and photos with add_image { asset } wherever the design has room for a picture. Never draw an imitation of a brand element (logo, underline, brush stroke, mascot, illustration) with text or shapes when the kit has the real one, and never invent quotes, people, prices or figures. If you use an image from outside the kit or leave one out, say so in your reply. An image the user gives you (attached to the chat, or a file you can read) goes into their uploads through upload_image or create_upload_link, then into the design with add_image { asset }; never leave it out silently.
Workflow: find or create a design (list_designs, search_templates, create_design) → read it with get_design → change it with edit_design (many operations per call), whose check field lists layout problems it found (text that doesn't fit or runs off the page, poor contrast, overlaps) → fix those, then look at the result with screenshot_page. Read get_design_guide once before your first edit.
Coordinates are px at 96 dpi from the page's top-left; colours are hex. The user may have the design open: they watch your changes live, and can undo them all (undo_ai_changes, or "Undo AI changes" in the editor). Share the editUrl so they can open it."#;

const GUIDE: &str = r#"# Designing in {brand}

## Model
- A design has pages of one size (get_design → size). Every element has a page-space box x, y, w, h in px (96 dpi), measured from the page's top-left. Rotation is in degrees around the box centre.
- Elements are stacked bottom to top in the order get_design lists them; new elements go on top. Use reorder to change it.
- Types: text, shape, line, image, frame (a photo placeholder with a clip shape), group, table, chart, qr, video, embed.
- Locked elements ("locked": true) were locked by the user; you cannot move or change them.

## Editing (edit_design)
- Send all the changes for a step in one call; they apply in order as one change. Give new elements a "ref" and use "$ref" later in the same call (for example to group, arrange or align what you just added).
- Text: add_text with w (the wrap width). Leave h out so the box fits the text. Style with font, size (px), weight (100–900), color, align, lineHeight, letterSpacing, case. update with "text" replaces the words and keeps the style.
- Images: add_image takes { url } (public https), { stock: { provider, id } } from search_media, { asset } from list_uploads, upload_image or an upload link, or { brand: { kit, name } } for a file in a brand kit by its file name (the name from the design system works). fit "cover" (default) crops to fill the box.
- Files from the user: upload_image takes the attachment itself where the chat app passes attachments to tools (ChatGPT does: pass the attached file as file), or a very small file (up to 64 KB, such as an SVG logo) as data_base64, because base64 passes through your context. Otherwise create_upload_link: if you can run commands or make HTTP requests, send the file to the link yourself (curl -T <file> "<url>"; a file attached to the chat is usually at /mnt/user-data/uploads/<name>) and the reply names the asset. If you can't, give the user the link: they drop the file on that page, and get_upload_link tells you the asset. Offer this rather than leaving an image out, and if an image still couldn't be included, say so plainly.
- Layout: arrange stacks items in a row or column or on a grid, with gap, align, justify and stretch; within "page" lays them out inside the page minus padding. align and distribute line items up with the page or with each other.
- Pages: add_page (optionally duplicate an existing page), update_page (background, title, speaker notes), move_page, delete_page.
- Text everywhere: replace_text swaps every match of find for replace across the design (or one page, or given elements) and keeps each word's styling. Use it to fill in a template's placeholder text, rename a product, or translate (one replace_text per sentence).
- Other formats: copy_design makes a copy, optionally resized to another preset (an Instagram story from a post, a banner, A4). Elements are repositioned to fit; fix what its check reports. Copy first when translating, so the original stays.
- Many versions from data: bulk_create makes one copy per row (certificates, badges, a post per product). Put placeholder text in the design first (for example "{name}"), then pass rows like { "{name}": "Ada" }; elements bound to a column take the value for that column name. Its warning lists fields that matched nothing.
- Shapes: {shapes}.
- Charts: {charts}.

## Good design
- Start from the brand kit (get_brand_kit) whose name matches the design system or brand the user selected or named, and use only that kit's assets; if several could match, ask. Create the design with create_design { brand_kit } so it is made where the kit's fonts live and get_design keeps showing the kit. Its voice holds the kit's written guidelines: follow them for every word. If the kit lists missingFonts, still use those fonts for their roles and tell the user to upload them. Use its colours, its fonts for each role, its logo, and its graphics and photos: each comes with an asset id and file name, so choose by name and check with screenshot_page (a logo that suits the background, for example). A design system in the chat gives you colours, fonts and rules, but its files can't reach {brand} from there, so use the kit's copies. Prefer a real image to a type-only layout when the format has room. Never draw an imitation of a brand element (logo, underline, brush stroke, mascot, illustration) with text or shapes when the kit has the real one: look through its graphics first. Use only facts the user or the brand gave you: never invent quotes, people, prices or figures, and mark anything you must assume as a placeholder and say so. If the kit lacks something the design needs, finish with what's there and say exactly what's missing. When a team's kit governs the design, get_design shows it as brand (fonts by role, colours); if brand.enforced is set, edit_design only accepts those colours and fonts.
- Give elements a role (headline, subhead, body, cta, logo, hero-image, price, legal, …) when you add them, and use the brand font for that role. Roles keep the hierarchy clear for later edits and resizes.
- Prefer a template (search_templates → create_design with template_id) for common formats; then replace its text (replace_text) and images.
- Give each page one clear focal point. Use size and weight for hierarchy (for example a 1080 px square post: headline 72–110 px, body 28–36 px).
- Keep margins generous (about 5–8% of the page width) and keep spacing consistent; align edges to each other.
- Use at most two font families. Keep strong contrast between text and its background (dark on light or light on dark; check with a screenshot).
- Presentations: one idea per slide, short text, large visuals; put the full script in speaker notes.
- Fix every problem edit_design reports in its check field, then judge the overall look with screenshot_page.
"#;

pub fn instructions(brand: &str) -> String {
    INSTRUCTIONS.replace("{brand}", brand)
}

pub fn guide(brand: &str) -> String {
    GUIDE.replace("{brand}", brand).replace("{shapes}", &oc_schema::elements::SHAPE_KINDS.join(", ")).replace("{charts}", &oc_schema::elements::CHART_KINDS.join(", "))
}
