ALTER TABLE meets
    ADD COLUMN IF NOT EXISTS venue_map_pdf_url TEXT,
    ADD COLUMN IF NOT EXISTS venue_map_apple_url TEXT;
