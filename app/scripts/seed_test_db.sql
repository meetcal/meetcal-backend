TRUNCATE TABLE
    lifting_results,
    session_schedule,
    athletes,
    saved_sessions,
    user_preferences,
    qualifying_totals,
    wso_records,
    records,
    standards,
    intl_rankings,
    meets
RESTART IDENTITY CASCADE;

INSERT INTO meets (
    convex_id,
    name,
    federation,
    start_date,
    end_date,
    status,
    time_zone,
    updated_at,
    venue_name,
    venue_street,
    venue_city,
    venue_state,
    venue_zip
) VALUES
    (
        'meet_ohio_2026',
        '2026 Ohio WSO Championships',
        'USAW',
        '2026-05-01',
        '2026-05-03',
        'completed',
        'America/New_York',
        1,
        'Ohio Expo Center',
        '717 E 17th Ave',
        'Columbus',
        'OH',
        '43211'
    ),
    (
        'meet_nationals_2026',
        '2026 USA Weightlifting National Championships, Powered by Rogue Fitness',
        'USAW',
        '2026-06-20',
        '2026-06-28',
        'upcoming',
        'America/Chicago',
        1,
        'Greater Columbus Convention Center',
        '400 N High St',
        'Columbus',
        'OH',
        '43215'
    );

INSERT INTO athletes (
    convex_id,
    member_id,
    name,
    age,
    club,
    wso,
    gender,
    weight_class,
    entry_total,
    session_number,
    session_platform,
    meet,
    adaptive
) VALUES
    (
        'athlete_kyle_schulman',
        '12345',
        'Kyle Schulman',
        27,
        'Vardanian Weightlifting',
        NULL,
        'Male',
        '+110',
        365,
        45,
        'Red',
        '2026 USA Weightlifting National Championships, Powered by Rogue Fitness',
        false
    ),
    (
        'athlete_columbus',
        '67890',
        'Columbus Test Athlete',
        24,
        'Columbus Weightlifting',
        'Ohio',
        'Women',
        '71kg',
        180,
        1,
        'Blue',
        '2026 Ohio WSO Championships',
        false
    );

INSERT INTO session_schedule (
    convex_id,
    date,
    session_id,
    start_time,
    weigh_in_time,
    platform,
    weight_class,
    meet
) VALUES
    (
        'schedule_nationals_45_red',
        '2026-06-20',
        45,
        '08:00:00',
        '06:00:00',
        'Red',
        '+110',
        '2026 USA Weightlifting National Championships, Powered by Rogue Fitness'
    );

INSERT INTO records (
    convex_id,
    record_type,
    age_category,
    gender,
    weight_class,
    snatch_record,
    cj_record,
    total_record
) VALUES
    ('record_usaw_men_senior_60', 'USAW', 'Senior', 'Men', '60kg', 100, 125, 225);

INSERT INTO standards (
    convex_id,
    age_category,
    gender,
    weight_class,
    standard_a,
    standard_b
) VALUES
    ('standard_men_senior_60', 'Senior', 'Men', '60kg', 281, 267);

INSERT INTO wso_records (
    convex_id,
    wso,
    age_category,
    gender,
    weight_class,
    snatch_record,
    cj_record,
    total_record
) VALUES
    ('wso_carolina_men_senior_60', 'Carolina', 'Senior', 'Men', '60', 101, 124, 225);

INSERT INTO qualifying_totals (
    convex_id,
    event_name,
    gender,
    age_category,
    weight_class,
    qualifying_total
) VALUES
    ('qt_virus_finals_women_u11_30', 'Virus Finals', 'Women', 'U11', '30kg', 30);

INSERT INTO intl_rankings (
    convex_id,
    legacy_id,
    meet,
    ranking,
    name,
    weight_class,
    total,
    percent_a,
    gender,
    age_category
) VALUES
    ('intl_worlds_women_junior', 1, 'Worlds', 1, 'Ella Nicholson', '77', 245, 114.49, 'Women', 'Junior');

INSERT INTO lifting_results (
    convex_id,
    legacy_id,
    event_id,
    meet,
    date,
    name,
    age,
    body_weight,
    snatch1,
    snatch2,
    snatch3,
    snatch_best,
    cj1,
    cj2,
    cj3,
    cj_best,
    total,
    adaptive,
    federation
) VALUES
    (
        'result_alexander_2025',
        1,
        'event_2025',
        '2025 Test Meet',
        '2025-06-01',
        'Alexander Nordstrom',
        'Open Men''s 60kg',
        59.9,
        90,
        95,
        100,
        100,
        120,
        125,
        130,
        130,
        230,
        false,
        'USAW'
    ),
    (
        'result_adaptive_2026',
        2,
        'event_2026',
        '2026 Adaptive Men 85kg National Championship',
        '2026-02-01',
        'Adaptive Test Athlete',
        'Adaptive Men 85kg',
        84.5,
        35,
        40,
        0,
        40,
        45,
        50,
        0,
        50,
        90,
        true,
        'USAW'
    );
