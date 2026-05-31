# User routes

Used by `hooks/useSavedSessions.ts` and `components/profile/AutoUnsaveSetting.tsx`.

Session ids are generated client-side as `{meet}-{sessionNumber}-{platform}` with spaces replaced by hyphens.

---

## `GET /users/me/saved-sessions`

**Why:** Sync saved sessions across devices after local AsyncStorage hydration.

**App usage:** `useSavedSessions.loadSavedSessions`

**Auth:** Clerk JWT

**Convex:** `savedSessions.getByUser` · `query` · `{ userId }` (from JWT subject)

**Response:** `200`

```json
{
  "sessions": [
    {
      "sessionId": "2025-Nationals-3-Red",
      "meet": "2025 Nationals",
      "sessionNumber": 3,
      "platform": "Red",
      "weightClass": "73kg",
      "startTime": "10:00 AM",
      "date": "2025-05-31",
      "notes": "optional",
      "athleteNames": ["Jane Doe"],
      "updatedAt": 1717171717000
    }
  ]
}
```

**Steps:**
1. Validate Clerk JWT; resolve `userId`.
2. Call Convex query; map rows to response shape.

---

## `PUT /users/me/saved-sessions/:sessionId`

**Why:** Persist a saved platform session for notifications and cross-device sync.

**App usage:** `useSavedSessions.saveSession`, `saveSessionsFromAthletes`

**Auth:** Clerk JWT

**Convex:** `savedSessions.upsert` · `mutation` · `{ sessionId, userId, meet, sessionNumber, platform, weightClass?, startTime?, notes?, athleteNames?, date? }`

**Request body:**

```json
{
  "meet": "2025 Nationals",
  "sessionNumber": 3,
  "platform": "Red",
  "weightClass": "73kg",
  "startTime": "10:00 AM",
  "date": "2025-05-31",
  "notes": "optional",
  "athleteNames": ["Jane Doe"]
}
```

**Response:** `200`

```json
{
  "sessionId": "2025-Nationals-3-Red",
  "updatedAt": 1717171717000
}
```

**Steps:**
1. Validate JWT; `userId` must match token subject.
2. Call Convex mutation with path `sessionId` + body fields.
3. Return id and timestamp.

**Notes:** App saves locally first; Convex/backend failure is non-fatal to the local save.

---

## `DELETE /users/me/saved-sessions/:sessionId`

**Why:** Remove one saved session.

**App usage:** `useSavedSessions.removeSession`

**Auth:** Clerk JWT

**Convex:** `savedSessions.remove` · `mutation` · `{ sessionId, userId }`

**Response:** `200`

```json
{ "deleted": true }
```

**Steps:**
1. Validate JWT.
2. Call Convex mutation; return `{ deleted: true|false }`.

---

## `DELETE /users/me/saved-sessions`

**Why:** Clear all saved sessions, or only those for one meet.

**App usage:** `useSavedSessions.resetAllSessions`

**Auth:** Clerk JWT

**Convex:** `savedSessions.removeAllForUser` · `mutation` · `{ userId, meet? }`

**Query params:** `meet` (optional) — meet name to scope deletion

**Response:** `200`

```json
{ "deletedCount": 4 }
```

**Steps:**
1. Validate JWT.
2. Call Convex mutation with optional `meet` query param.
3. Return deleted count from mutation result.

---

## `GET /users/me/preferences`

**Why:** Load auto-unsave setting on profile screen.

**App usage:** `AutoUnsaveSetting` (premium feature)

**Auth:** Clerk JWT

**Convex:** `userPreferences.getForCurrentUser` · `query` · `{}`

**Response:** `200`

```json
{
  "autoUnsaveStartedSessions": false
}
```

**Steps:**
1. Validate JWT (Convex reads identity from forwarded Clerk token).
2. Return query result.

---

## `PATCH /users/me/preferences/auto-unsave`

**Why:** Toggle automatic removal of saved sessions 2 hours after session start.

**App usage:** `AutoUnsaveSetting.handleToggle` (requires subscription in app UI)

**Auth:** Clerk JWT

**Convex:** `userPreferences.setAutoUnsaveStartedSessions` · `mutation` · `{ enabled }`

**Request body:**

```json
{ "enabled": true }
```

**Response:** `200`

```json
{ "autoUnsaveStartedSessions": true }
```

**Steps:**
1. Validate JWT.
2. Call Convex mutation.
3. Return new value.

**Notes:** Expiry cleanup stays in Convex (`autoUnsaveSavedSessionsJob.run` cron in `convex/crons.ts`).
