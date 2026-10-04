<!--
SPDX-FileCopyrightText: 2026 Gabriel Piñones
SPDX-License-Identifier: AGPL-3.0-or-later
-->

# Baze

Navegación colaborativa en bicicleta con reportes comunitarios en tiempo real, mapa offline y privacidad por diseño.

## 1. Visión del Proyecto

Baze es una plataforma de navegación ciclista enfocada en Android (con planes futuros para iOS), construida sobre estándares abiertos y software libre:
- **Monolito modular en Rust**: Único punto de entrada a la API, sin dependencias de microservicios ni intermediarios innecesarios.
- **Ruteo adaptativo con Valhalla**: Rutas optimizadas para ciclistas considerando pendientes y elevación, evitando incidentes bloqueantes confirmados por la comunidad.
- **Reportes colaborativos**: Advertencias inmediatas (`warning`) y bloqueos confirmados (`blocking`) validados por la comunidad.
- **Privacidad y sin telemetría**: No se almacenan trazas GPS ni posiciones continuas del usuario. Identidad anónima desde el primer uso.
- **Mapas vectoriales autónomos**: Descarga de PMTiles locales para funcionamiento offline sin depender de APIs de terceros.
- **100% Libre y sin Google Play Services / Firebase**: Preparado para distribución libre (F-Droid).

## 2. Estructura del Repositorio

- [`backend/`](backend/): Monolito modular en Rust (Axum, Tokio, SQLx, PostGIS, utoipa).
- [`android/`](android/): App Android en Kotlin con Jetpack Compose y MapLibre Native.
- [`infra/`](infra/): Orquestación con Docker Compose y Caddy como reverse proxy TLS.
- [`data/`](data/): Pipeline unificado para compilar PMTiles, grafo Valhalla e índice Photon desde el mismo extracto OSM.
- [`contracts/`](contracts/): Definición de contratos OpenAPI (`openapi.json`) generada desde el backend.
- [`docs/`](docs/): Arquitectura, guía de operación y Registros de Decisiones de Arquitectura (ADR).
- [`scripts/`](scripts/): Utilidades de desarrollo y CI (generación de `.env`, validación de Compose, regresión de seguridad de extremo a extremo).

## 3. Primeros Pasos

Requisitos: Docker con Compose, `make` y, para desarrollar el backend, la toolchain de Rust que fija `backend/rust-toolchain.toml` (la instala `rustup` al ejecutar cualquier `cargo` dentro de `backend/`).

```bash
# 1. Crea infra/.env con secretos aleatorios (no sobrescribe uno existente) y levanta la pila:
#    PostGIS, migraciones, backend y Caddy. El backend queda en http://127.0.0.1:8080
make dev-up

# 2. Comprueba que responde
curl http://127.0.0.1:8080/health

# 3. Detén los servicios
make dev-down
```

Ruteo y geocodificación responden `501` mientras `ENGINES_ENABLED` esté apagado (el valor por defecto); el resto de la API (cuentas, reportes, votos y tiempo real) funciona. Valhalla y Photon necesitan datos generados con `make data-build`, se levantan con `--profile engines` y se usan con `ENGINES_ENABLED=true` (ver [`data/README.md`](data/README.md)).

## 4. Comandos Principales

El repositorio incluye un `Makefile` para estandarizar las tareas de desarrollo (`make help` los lista):

```bash
make dev-env         # Crea infra/.env con secretos aleatorios
make dev-up          # Levanta la pila de desarrollo (backend en 127.0.0.1:8080)
make dev-down        # Detiene la pila

make backend-check   # Formato, clippy sin advertencias, pruebas y contrato OpenAPI al día
make backend-db-test # Todas las pruebas, incluidas las de PostgreSQL (necesita DATABASE_URL)
make backend-deny    # Licencias, fuentes y avisos de seguridad de las dependencias de Rust
make compose-check   # Valida la pila de Compose y sus invariantes de seguridad
make openapi         # Regenera contracts/openapi.json desde el código

make android-build   # Compila la app Android (debug)
make data-build      # Pipeline de datos OSM (PMTiles, Valhalla, Photon)
```

Para operar un despliegue (secretos, rotación, moderación, configuración) consulta [`docs/operations.md`](docs/operations.md). Las decisiones de diseño están en [`docs/adr/`](docs/adr/).

## 5. Licencia y Atribución

- Código: [GNU Affero General Public License v3.0 or later (AGPL-3.0-or-later)](LICENSE).
- Datos cartográficos: © [OpenStreetMap contributors](https://www.openstreetmap.org/copyright) bajo licencia [ODbL](https://opendatacommons.org/licenses/odbl/). Ver [ATTRIBUTION.md](ATTRIBUTION.md).
- Política de contribuciones y CLA: Ver [CONTRIBUTING.md](CONTRIBUTING.md).
