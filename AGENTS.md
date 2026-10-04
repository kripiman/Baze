<!--
SPDX-FileCopyrightText: 2026 Gabriel Piñones
SPDX-License-Identifier: AGPL-3.0-or-later
-->

# AGENTS.md — Contexto Único para Agentes de Código

Este documento es la **fuente de verdad canónica** sobre la arquitectura, reglas de diseño, comandos y restricciones del proyecto **Baze**. Todo agente o modelo de IA que trabaje en este monorepo debe acatar estrictamente estas directrices.

---

## 1. Resumen del Sistema

**Baze** es una plataforma de navegación en bicicleta con reportes colaborativos (estilo Waze) y privacidad por diseño.
- **Plataformas**: Android nativo en la fase inicial; planeado para migración a Kotlin Multiplatform e iOS.
- **Filosofía**: Monolito modular en Rust. Sin microservicios, sin Kafka, sin Redis, sin Kubernetes.
- **Mapa y Ruteo**: Vector tiles servidos/leídos localmente vía PMTiles (`pmtiles://file://`). Ruteo ciclista con elevación mediante Valhalla. Geocodificación con Photon a través del backend.
- **Red y Seguridad**: Caddy es el único componente con puertos públicos en producción. Servicios internos residen en red Docker aislada (`internal: true`).

---

## 2. Mapa del Monorepo

```text
baze/
├── AGENTS.md                 # Esta guía canónica
├── README.md                 # Descripción general y primeros pasos
├── LICENSE                   # GNU AGPLv3 oficial
├── ATTRIBUTION.md            # Atribuciones a OSM, ODbL y librerías
├── CONTRIBUTING.md           # Guía de contribución y CLA
├── SECURITY.md               # Política y reporte privado de seguridad
├── Makefile                  # Tareas de automatización (dev-up, backend-check, etc.)
├── .editorconfig             # Reglas de formato unificadas
├── .gitignore                # Reglas de exclusión de git
├── .gitattributes            # Normalización LF y binarios
├── scripts/                  # check-toolchain-sync, compose-check, check-style, gen-env y e2e/ (regresión de abuso y smoke de motores)
├── .github/
│   ├── CODEOWNERS, dependabot.yml, PULL_REQUEST_TEMPLATE.md, ISSUE_TEMPLATE/
│   └── workflows/
│       ├── backend.yml       # CI de Rust: fmt, clippy -D warnings, tests con PostGIS, e2e, contrato OpenAPI, cargo-deny, MSRV 1.88
│       ├── infra.yml         # CI de infraestructura: compose, Caddyfile, shellcheck, estilo del mapa y pila real con sus invariantes
│       ├── data-smoke.yml    # Pipeline de datos y motores reales (Valhalla, Photon) sobre Mónaco, a través del backend
│       ├── android.yml       # CI de Android: lint, test, checkPurity, assembleDebug/Release (R8), licencias
│       ├── security.yml      # Avisos RustSec y crates retirados (programado)
│       └── secrets.yml       # Auditoría de secretos con Gitleaks
├── contracts/
│   └── openapi.json          # Contrato OpenAPI generado desde el backend con utoipa
├── backend/                  # Monolito modular en Rust (edition 2024)
│   ├── Cargo.toml            # Workspace raíz
│   ├── Cargo.lock            # Versionado
│   ├── rust-toolchain.toml   # Versión y componentes del toolchain
│   ├── deny.toml             # Allowlist de licencias y seguridad
│   ├── Dockerfile            # Multi-stage con cargo-chef y usuario no root
│   ├── migrations/           # Migraciones SQLx (PostGIS); las aplicadas nunca se editan
│   └── crates/
│       ├── app/              # Binario ejecutable, wiring, Axum router, /health, /source
│       ├── shared/           # Tipos GeoJSON, errores y traits de dominio (SIN IO)
│       ├── auth/             # Cuentas anónimas y gestión de tokens
│       ├── hazards/          # Reportes, votos comunitarios, confirmación y TTL
│       ├── routing/          # Cliente Valhalla y regla de re-ruteo con exclude_polygons
│       ├── geocoding/        # Cliente proxy a Photon
│       └── realtime/         # SSE de peligros por área geográfica
├── android/                  # Aplicación móvil Android
│   ├── settings.gradle.kts   # Configuración Gradle multi-módulo
│   ├── build.gradle.kts      # Raíz de build Gradle
│   ├── gradle/libs.versions.toml # Catálogo unificado de dependencias
│   ├── build-logic/          # Convention plugins de Gradle
│   ├── app/                  # Application, MainActivity, navegación Compose, Koin
│   ├── core/
│   │   ├── model/            # Kotlin puro (sin android.* ni java.*)
│   │   ├── domain/           # Kotlin puro (casos de uso y proximidad)
│   │   ├── network/          # Ktor Client + cliente generado desde openapi.json
│   │   └── location/         # Interfaz y provider basado en LocationManager
│   └── feature/
│       ├── map/              # MapLibre Native Android + PMTiles local
│       ├── search/           # Búsqueda de direcciones vía Photon
│       ├── navigation/       # Foreground service de navegación + conexión SSE
│       └── reports/          # Creación y votación de reportes
├── infra/                    # Despliegue en VPS
│   ├── compose.yaml          # Orquestación (Caddy, Backend, PostGIS; Valhalla y Photon en el perfil `engines`)
│   ├── compose.dev.yaml      # Superposición solo para desarrollo (puertos en 127.0.0.1)
│   ├── caddy/Caddyfile       # Configuración TLS y proxy reverso
│   ├── postgres/init/        # Roles de BD (propietario que migra, `baze_app` sin DDL)
│   ├── photon/Dockerfile     # Imagen propia de Photon (jar de la release verificado por sha256)
│   └── .env.example          # Plantilla de variables de entorno (sin secretos; `make dev-env` los genera)
├── data/                     # Pipeline de ingestión y compilación de datos OSM
│   ├── README.md             # Instrucciones paso a paso
│   ├── scripts/              # 01-download, 02-pmtiles, 03-valhalla (+ container/), 04-photon, 05-publish-static
│   ├── fixtures/             # Volcado mínimo de Photon (Mónaco) para el smoke test
│   ├── styles/               # style.json de MapLibre (marcador {{PMTILES_URL}}, sin sprites ni glifos)
│   └── out/                  # Artefactos compilados (.pmtiles, grafo, índice)
└── docs/
    ├── architecture.md       # Diagrama y detalle de reglas de dominio
    ├── operations.md         # Despliegue, secretos, copias, rotación y verificación
    ├── ESTADO-DEL-PROYECTO.md # Traspaso: qué está hecho, qué falta y cómo retomar
    └── adr/                  # Architecture Decision Records (0001-0008)
```

---

## 3. Comandos de Build, Test y Lint

### Backend (Rust)
```bash
# Formato de código
cargo fmt --all --check

# Linter de código estricto
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings

# Pruebas unitarias y de integración (sin base de datos)
cargo test --locked --workspace

# Con PostgreSQL+PostGIS desechable (sqlx::test crea una base por prueba; nunca apuntar a datos reales)
DATABASE_URL=postgres://postgres:postgres@127.0.0.1:5432/postgres cargo test --locked --workspace --all-features

# Verificación de licencias y vulnerabilidades
cargo deny check

# Contrato OpenAPI: regenerar y verificar que está al día
cargo run --locked -p baze-app --bin export-openapi            # escribe contracts/openapi.json
cargo run --locked -p baze-app --bin export-openapi -- --check
```

Los atajos equivalentes están en el `Makefile`: `make backend-check` (fmt, clippy, tests y contrato), `make backend-db-test`, `make backend-deny`, `make toolchain-check`, `make openapi`, `make compose-check`. Todos se ejecutan desde la raíz; las recetas entran en `backend/` porque `rustup` resuelve `rust-toolchain.toml` desde el directorio de trabajo.

### Android (Kotlin / Gradle)
```bash
# Compilación en modo Debug
cd android && ./gradlew assembleDebug

# Pruebas unitarias
cd android && ./gradlew test

# Linter de Android
cd android && ./gradlew lint

# Pureza de core/model y core/domain (sin android.* ni java.*)
cd android && ./gradlew checkPurity

# Verificación de licencias de dependencias (el CI lo corre junto con lint, test y assembleRelease)
cd android && ./gradlew checkLicenses
```

### Infraestructura y Datos
```bash
# Validar docker compose y sus invariantes de seguridad (no arranca nada; usa valores falsos)
bash scripts/compose-check.sh

# Regresión de abuso contra el binario real (necesita PostgreSQL; ver la cabecera del script)
url="$(bash scripts/e2e/prepare-db.sh "$DATABASE_URL")"
python3 scripts/e2e/audit_regression.py --database-url "$url" --admin-database-url "$DATABASE_URL" --through 10

# Ejecutar el pipeline de datos (desde la raíz; también `make data-build`). El 04 exige una fuente (ver data/README.md)
bash data/scripts/01-download-extract.sh
bash data/scripts/02-build-pmtiles.sh
bash data/scripts/03-build-valhalla.sh
PHOTON_IMPORT_FILE=data/fixtures/photon-monaco.jsonl bash data/scripts/04-build-photon.sh
bash data/scripts/05-publish-static.sh

# Estilo del mapa (también `make style-check`)
python3 scripts/check-style.py
```

---

## 4. Reglas Arquitectónicas y de Dominio

### Backend
1. **Aislamiento de Crates de Dominio**: Los crates `auth`, `hazards`, `routing`, `geocoding` y `realtime` dependen **únicamente de `shared`**. Nunca dependen entre sí. Si `routing` requiere consultar bloqueos, consume un trait definido en `shared` e implementado en `hazards`. El crate `app` realiza la inyección de dependencias (wiring).
2. **Handlers Delgados**: Los controladores web en Axum solo deserializan, validan entradas, delegan a la capa de dominio y devuelven respuestas.
3. **Geometría en la API**: Formato GeoJSON en la capa HTTP. En SQL se usa `ST_GeomFromGeoJSON` y `ST_AsGeoJSON`.
4. **Cumplimiento AGPLv3**: El endpoint `GET /source` debe retornar la URL del repositorio Git y el hash del commit desplegado.
5. **Configuración**: Exclusivamente mediante variables de entorno validadas al arranque.

### Android
1. **Pureza de `core/model` y `core/domain`**: Estrictamente **Kotlin puro**. Prohibido importar paquetes `android.*` o `java.*`. Solo se permite Kotlin stdlib y `kotlinx.*`.
2. **Cero Dependencias Privativas**: Prohibido el uso de Google Play Services, Firebase o librerías que requieran servicios de Google. El proveedor de ubicación debe usar `android.location.LocationManager`.
3. **Servicio en Primer Plano**: La navegación GPS y la suscripción SSE corren dentro de un `Service` en primer plano con `foregroundServiceType="location"`.
4. **Almacenamiento Local de Mapas**: La app descarga el archivo `.pmtiles` y lo lee con protocolo local `pmtiles://file://`.

### Dominio y Ruteo
1. **Clasificación de Reportes**:
   - `warning` (vidrio, bache, calzada irregular): solo genera alertas visuales/sonoras.
   - `blocking` (calle cortada, obra, inundación): afecta el ruteo **solo cuando está confirmado** por el umbral de votos comunitarios.
2. **Evitación en Valhalla**: El backend solicita la ruta base a Valhalla, comprueba en PostGIS si la geometría interseca bloqueos confirmados, y solo ante intersecciones relanza la solicitud enviando `exclude_polygons`; la ruta nueva se comprueba otra vez. **Fail-closed**: una ruta que cruza un cierre confirmado nunca se entrega (si no hay alternativa, `404`; si hay demasiados cierres, `503`).
3. **Alertas y SSE**: El servidor entrega peligros cercanos a la ruta en la respuesta inicial. El stream SSE solo notifica eventos nuevos o cambiados en el bounding box de la ruta activa (`event: hazard`). Un cliente que se queda atrás recibe `event: resync` y debe volver a pedir `GET /api/v1/hazards`: nunca se pierden eventos en silencio.
4. **Expiración**: Los reportes se descartan mediante `expires_at` en toda consulta SQL y un job periódico purga registros caducados.

---

## 5. Qué NO Hacer (Prohibiciones Estrictas)

- **NO** implementar microservicios, Kafka, Redis, RabbitMQ ni clústeres de Kubernetes.
- **NO** almacenar ni transmitir trazas GPS continuas (telemetría) de los usuarios.
- **NO** exponer puertos de PostGIS, Valhalla o Photon en interfaces públicas de Internet.
- **NO** utilizar imágenes Docker con tag `latest`. Todas las imágenes deben tener versiones fijadas (las de terceros, por digest).
- **NO** registrar texto de búsqueda ni coordenadas de rutas en ningún log (backend, Valhalla, Photon, Caddy).
- **NO** incluir credenciales, secretos, keystores ni archivos `.env` en el repositorio.
- **NO** acoplar crates de dominio entre sí.

---

## 6. Procedimiento de Verificación de Cambios

Antes de considerar una tarea completada:
1. Backend: `make backend-check` (formato, clippy `-D warnings`, tests y contrato OpenAPI al día). Si el cambio toca la base de datos o la votación, también `make backend-db-test` con un PostgreSQL+PostGIS desechable.
2. Si cambian las rutas, los esquemas o la documentación de la API: `make openapi` y commitear `contracts/openapi.json`.
3. Validar Docker Compose y shell: `bash scripts/compose-check.sh` (`make compose-check`) y `shellcheck` sobre `scripts/` y `data/scripts/`.
4. Si el SDK de Android está configurado: `cd android && ./gradlew lint test checkPurity assembleDebug`.
5. Si cambian `data/`, `infra/photon/` o los clientes de los motores: el workflow `data-smoke.yml` (Mónaco, motores reales) debe pasar.
6. Tras empujar, comprobar que los workflows de GitHub pasan (backend, infra, android, data-smoke, secrets, security).
7. Confirmar que no se hayan generado archivos no deseados fuera de `.gitignore`.
