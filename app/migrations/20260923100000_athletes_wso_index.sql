-- `/wsos/athletes` filters `athletes` by `wso`; the table only had meet, club,
-- and member_id indexes, so every WSO history request was a sequential scan.
CREATE INDEX IF NOT EXISTS idx_athletes_wso ON athletes (wso);
