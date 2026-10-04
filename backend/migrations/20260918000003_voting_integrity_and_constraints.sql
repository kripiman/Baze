-- SPDX-FileCopyrightText: 2026 Gabriel Piñones
-- SPDX-License-Identifier: AGPL-3.0-or-later

-- Integridad de los reportes: el servidor ya valida todo esto, la base de datos lo impone también
-- para que un bug o un acceso directo no pueda dejar filas incoherentes.

-- Los contadores se derivan de las papeletas (hazard_votes); el voto del creador es una papeleta más.
ALTER TABLE hazards ALTER COLUMN upvotes SET DEFAULT 0;

ALTER TABLE hazards
    ADD CONSTRAINT ck_hazards_category CHECK (
        category IN ('glass', 'pothole', 'debris', 'road_closed', 'construction', 'flood')
    ),
    ADD CONSTRAINT ck_hazards_type CHECK (hazard_type IN ('warning', 'blocking')),
    ADD CONSTRAINT ck_hazards_status CHECK (status IN ('unconfirmed', 'confirmed', 'resolved')),
    -- El tipo lo deriva el servidor de la categoría; un par incoherente no debe existir nunca.
    ADD CONSTRAINT ck_hazards_category_type CHECK (
        (category IN ('glass', 'pothole', 'debris') AND hazard_type = 'warning')
        OR (category IN ('road_closed', 'construction', 'flood') AND hazard_type = 'blocking')
    ),
    ADD CONSTRAINT ck_hazards_counters CHECK (upvotes >= 0 AND downvotes >= 0),
    ADD CONSTRAINT ck_hazards_expiry CHECK (expires_at > created_at),
    ADD CONSTRAINT ck_hazards_description CHECK (description IS NULL OR char_length(description) <= 500);

-- Consultas de corredor: ST_DWithin(geom::geography, ruta::geography, metros)
CREATE INDEX IF NOT EXISTS idx_hazards_geog ON hazards USING GIST ((geom::geography));
-- Topes diarios por cuenta
CREATE INDEX IF NOT EXISTS idx_hazards_creator_created ON hazards (creator_account_id, created_at);

-- Consenso multi-red (ADR-0006): solo el primer voto de cada subred cuenta para el umbral.
--   counts    : si la papeleta cuenta (la primera de su subred para ese reporte)
--   voter_net : etiqueta con clave (HMAC-SHA-256 truncado a 128 bits) de la subred del votante.
--               NUNCA una dirección IP: la base de datos no guarda trazas de red (ADR-0005/0008).
ALTER TABLE hazard_votes ADD COLUMN IF NOT EXISTS counts BOOLEAN NOT NULL DEFAULT TRUE;
ALTER TABLE hazard_votes ADD COLUMN IF NOT EXISTS voter_net BYTEA;

-- Cualquier fila anterior a esta migración no tiene subred conocida: deja de contar (falla de forma segura).
UPDATE hazard_votes
SET voter_net = decode(repeat('00', 16), 'hex'), counts = FALSE
WHERE voter_net IS NULL;

ALTER TABLE hazard_votes ALTER COLUMN voter_net SET NOT NULL;
ALTER TABLE hazard_votes ADD CONSTRAINT ck_hazard_votes_net CHECK (octet_length(voter_net) = 16);

COMMENT ON COLUMN hazard_votes.voter_net IS
    'HMAC-SHA-256 (truncated to 128 bits) of the voter''s /24 or /64 network under a server key. Never an IP address.';

-- A lo sumo una papeleta que cuenta por subred y reporte; la aplicación lo respeta, esto es la red de seguridad.
CREATE UNIQUE INDEX IF NOT EXISTS uq_hazard_votes_counted_net
    ON hazard_votes (hazard_id, voter_net) WHERE counts;

-- Topes diarios de votos por cuenta
CREATE INDEX IF NOT EXISTS idx_hazard_votes_account_created ON hazard_votes (account_id, created_at);
