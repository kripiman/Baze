-- SPDX-FileCopyrightText: 2026 Gabriel Piñones
-- SPDX-License-Identifier: AGPL-3.0-or-later

-- Cuentas anónimas
CREATE TABLE IF NOT EXISTS accounts (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    is_active BOOLEAN NOT NULL DEFAULT TRUE
);

-- Reportes viales comunitarios
CREATE TABLE IF NOT EXISTS hazards (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    creator_account_id UUID NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    category VARCHAR(50) NOT NULL, -- glass, pothole, debris, road_closed, construction, flood
    hazard_type VARCHAR(20) NOT NULL, -- warning | blocking
    status VARCHAR(20) NOT NULL DEFAULT 'unconfirmed', -- unconfirmed | confirmed | resolved
    description TEXT,
    upvotes INTEGER NOT NULL DEFAULT 1,
    downvotes INTEGER NOT NULL DEFAULT 0,
    geom GEOMETRY(Point, 4326) NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at TIMESTAMPTZ NOT NULL
);

-- Índices para consultas espaciales y de ciclo de vida
CREATE INDEX IF NOT EXISTS idx_hazards_geom ON hazards USING GIST (geom);
CREATE INDEX IF NOT EXISTS idx_hazards_expires_at ON hazards (expires_at);
CREATE INDEX IF NOT EXISTS idx_hazards_type_status ON hazards (hazard_type, status);

-- Votos comunitarios (regla: un voto por cuenta por reporte)
CREATE TABLE IF NOT EXISTS hazard_votes (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    hazard_id UUID NOT NULL REFERENCES hazards(id) ON DELETE CASCADE,
    account_id UUID NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    vote_type SMALLINT NOT NULL CHECK (vote_type IN (1, -1)),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT uq_hazard_account_vote UNIQUE (hazard_id, account_id)
);

CREATE INDEX IF NOT EXISTS idx_hazard_votes_hazard_id ON hazard_votes (hazard_id);
