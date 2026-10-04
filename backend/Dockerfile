# SPDX-FileCopyrightText: 2026 Gabriel Piñones
# SPDX-License-Identifier: AGPL-3.0-or-later

# Debe coincidir con backend/rust-toolchain.toml y .github/workflows/backend.yml
# (lo verifica scripts/check-toolchain-sync.sh).
ARG RUST_VERSION=1.97.1

# Stage 1: Preparación con cargo-chef (versión fijada)
FROM lukemathwalker/cargo-chef:0.1.78-rust-${RUST_VERSION}-bookworm AS chef
WORKDIR /app

# Stage 2: Generación del recipe de dependencias
FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

# Stage 3: Compilación con caché de dependencias
FROM chef AS builder
COPY --from=planner /app/recipe.json recipe.json
# Compilar solo las dependencias externas para maximizar el uso de caché
RUN cargo chef cook --release --locked --recipe-path recipe.json

# Compilar el código fuente del proyecto
COPY . .
RUN cargo build --release --locked --bin baze-server

# Stage 4: Imagen de ejecución mínima con usuario sin privilegios
FROM debian:bookworm-slim AS runtime

# El binario enlaza solo libc/libm/libgcc_s y no hace llamadas TLS salientes:
# no hacen falta ni ca-certificates ni libssl.

# Crear usuario y grupo no root, sin shell de login ni directorio home
RUN groupadd -g 10001 baze && \
    useradd -u 10001 -g baze -s /usr/sbin/nologin -M baze

WORKDIR /app

# Copiar el binario compilado
COPY --from=builder /app/target/release/baze-server /usr/local/bin/baze-server

# Hash del commit con el que se construye la imagen (AGPL, sección 13: /source debe apuntar al código
# desplegado). Se declara aquí, tras la compilación, para no invalidar la caché de los stages anteriores.
# En producción el backend se niega a arrancar si no es un hash hexadecimal válido.
ARG GIT_COMMIT_HASH=
ENV GIT_COMMIT_HASH=${GIT_COMMIT_HASH}

# Variables de entorno por defecto
ENV PORT=8080
ENV HOST=0.0.0.0
ENV ENVIRONMENT=production

# Cambiar a usuario no privilegiado
USER baze:baze

EXPOSE 8080

# La imagen no trae curl ni shell útil: el propio binario hace la comprobación.
HEALTHCHECK --interval=15s --timeout=3s --start-period=10s --retries=3 \
    CMD ["/usr/local/bin/baze-server", "healthcheck"]

ENTRYPOINT ["/usr/local/bin/baze-server"]
