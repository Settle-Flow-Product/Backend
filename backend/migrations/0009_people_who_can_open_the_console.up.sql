-- Who may open the operations console.
--
-- Separate from everything else in this schema, because it is the only table that describes people
-- rather than payments. It exists for `frontend/admin`; the backend never reads it.

CREATE TABLE admin_users (
    id            uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    email         text NOT NULL UNIQUE,
    password_hash text NOT NULL,

    -- An array rather than a single role: somebody covering two jobs holds two roles, and the
    -- alternative is inventing a compound role for every pair.
    roles         text[] NOT NULL DEFAULT ARRAY['viewer'],

    active        boolean NOT NULL DEFAULT true,

    -- Bumped by suspending an account, changing its password, or editing its roles.
    --
    -- A session token carries the epoch it was minted at, so all three take effect AT ONCE rather
    -- than at the end of an eight-hour shift. Without it, revoking access means waiting for a token
    -- to expire — which is not revoking access.
    token_epoch   integer NOT NULL DEFAULT 1,

    created_at    timestamptz NOT NULL DEFAULT now(),
    updated_at    timestamptz NOT NULL DEFAULT now(),

    CONSTRAINT roles_are_known CHECK (
        roles <@ ARRAY['superadmin', 'admin', 'operator', 'viewer']::text[]),
    CONSTRAINT at_least_one_role CHECK (cardinality(roles) >= 1)
);

CREATE INDEX admin_users_active ON admin_users (email) WHERE active;
