UPDATE qualifying_totals
SET gender = INITCAP(LOWER(gender))
WHERE gender <> INITCAP(LOWER(gender));

UPDATE qualifying_totals
SET age_category = INITCAP(LOWER(age_category))
WHERE age_category <> INITCAP(LOWER(age_category));

UPDATE athletes
SET gender = INITCAP(LOWER(gender))
WHERE gender <> INITCAP(LOWER(gender));
