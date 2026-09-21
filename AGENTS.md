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
├── .github/workflows/
│   ├── backend.yml           # CI de Rust (fmt, clippy, test, cargo-deny)
│   ├── android.yml           # CI de Android (lint, test, assembleDebug, licencias)
│   └── secrets.yml           # Auditoría de secretos con Gitleaks
├── contracts/
│   └── openapi.json          # Contrato OpenAPI generado desde el backend con utoipa
├── backend/                  # Monolito modular en Rust (edition 2024)
│   ├── Cargo.toml            # Workspace raíz
│   ├── Cargo.lock            # Versionado
│   ├── rust-toolchain.toml   # Versión y componentes del toolchain
│   ├── deny.toml             # Allowlist de licencias y seguridad
│   ├── Dockerfile            # Multi-stage con cargo-chef y usuario no root
│   ├── migrations/           # Migraciones SQLx (PostGIS)
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
│   ├── compose.yaml          # Orquestación (Caddy, Backend, PostGIS, Valhalla, Photon)
│   ├── caddy/Caddyfile       # Configuración TLS y proxy reverso
│   └── .env.example          # Plantilla de variables de entorno
├── data/                     # Pipeline de ingestión y compilación de datos OSM
│   ├── README.md             # Instrucciones paso a paso
│   ├── scripts/              # 01-download, 02-pmtiles, 03-valhalla, 04-photon
│   ├── styles/               # style.json de MapLibre, sprites y glifos
│   └── out/                  # Artefactos compilados (.pmtiles, grafo, índice)
└── docs/
    ├── architecture.md       # Diagrama y detalle de reglas de dominio
    └── adr/                  # Architecture Decision Records
```

---

## 3. Comandos de Build, Test y Lint

### Backend (Rust)
```bash
# Formato de código
cargo fmt --all --check

# Linter de código estricto
cargo clippy --workspace --all-targets -- -D warnings

# Pruebas unitarias y de integración
cargo test --workspace

# Verificación de licencias y vulnerabilidades
cargo deny check

# Generación del contrato OpenAPI
cargo run -p baze-app --bin export-openapi
```

### Android (Kotlin / Gradle)
```bash
# Compilación en modo Debug
cd android && ./gradlew assembleDebug

# Pruebas unitarias
cd android && ./gradlew test

# Linter de Android
cd android && ./gradlew lint

# Verificación de licencias de dependencias
cd android && ./gradlew checkLicenses
```

### Infraestructura y Datos
```bash
# Validar sintaxis de docker compose
docker compose -f infra/compose.yaml config

# Ejecutar el pipeline de datos (desde la raíz)
bash data/scripts/01-download-extract.sh
bash data/scripts/02-build-pmtiles.sh
bash data/scripts/03-build-valhalla.sh
bash data/scripts/04-build-photon.sh
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
2. **Evitación en Valhalla**: El backend solicita la ruta base a Valhalla, comprueba en PostGIS si la geometría interseca bloqueos confirmados, y solo ante intersecciones relanza la solicitud enviando `exclude_polygons`.
3. **Alertas y SSE**: El servidor entrega peligros cercanos a la ruta en la respuesta inicial. El stream SSE solo notifica eventos nuevos en el bounding box de la ruta activa.
4. **Expiración**: Los reportes se descartan mediante `expires_at` en toda consulta SQL y un job periódico purga registros caducados.

---

## 5. Qué NO Hacer (Prohibiciones Estrictas)

- **NO** implementar microservicios, Kafka, Redis, RabbitMQ ni clústeres de Kubernetes.
- **NO** almacenar ni transmitir trazas GPS continuas (telemetría) de los usuarios.
- **NO** exponer puertos de PostGIS, Valhalla o Photon en interfaces públicas de Internet.
- **NO** utilizar imágenes Docker con tag `latest`. Todas las imágenes deben tener versiones fijadas.
- **NO** incluir credenciales, secretos, keystores ni archivos `.env` en el repositorio.
- **NO** acoplar crates de dominio entre sí.

---

## 6. Procedimiento de Verificación de Cambios

Antes de considerar una tarea completada:
1. Validar compilación de backend: `cargo check --workspace --all-targets`.
2. Validar que no existan advertencias de clippy: `cargo clippy --workspace --all-targets -- -D warnings`.
3. Verificar formato: `cargo fmt --all -- --check`.
4. Validar Docker Compose: `docker compose -f infra/compose.yaml config`.
5. Si el SDK de Android está configurado: `cd android && ./gradlew assembleDebug`.
6. Confirmar que no se hayan generado archivos no deseados fuera de `.gitignore`.
