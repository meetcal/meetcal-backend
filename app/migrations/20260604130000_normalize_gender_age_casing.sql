UPDATE intl_rankings
SET
    gender = INITCAP(LOWER(gender))
WHERE gender IS NOT NULL AND gender <> INITCAP(LOWER(gender));

UPDATE intl_rankings
SET
    age_category = INITCAP(LOWER(age_category))
WHERE age_category IS NOT NULL AND age_category <> INITCAP(LOWER(age_category));

UPDATE standards
SET
    gender = INITCAP(LOWER(gender))
WHERE gender <> INITCAP(LOWER(gender));

UPDATE standards
SET
    age_category = INITCAP(LOWER(age_category))
WHERE age_category <> INITCAP(LOWER(age_category));

UPDATE records
SET
    gender = INITCAP(LOWER(gender))
WHERE gender <> INITCAP(LOWER(gender));

UPDATE records
SET
    age_category = INITCAP(LOWER(age_category))
WHERE age_category <> INITCAP(LOWER(age_category));

UPDATE wso_records
SET
    gender = INITCAP(LOWER(gender))
WHERE gender <> INITCAP(LOWER(gender));

UPDATE wso_records
SET
    age_category = INITCAP(LOWER(age_category))
WHERE age_category <> INITCAP(LOWER(age_category));
