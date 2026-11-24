-- An answer to an <Ask> that names cards (`<Ask cards="c_12">`) belongs to the thread of the default that names them.
ALTER TABLE report_answers ADD COLUMN cards TEXT;
