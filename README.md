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
- [`docs/`](docs/): Documentación de arquitectura y Registros de Decisiones de Arquitectura (ADR).

## 3. Comandos Principales

El repositorio incluye un `Makefile` para estandarizar las tareas de desarrollo:

```bash
# Iniciar servicios de infraestructura en desarrollo (PostGIS, Valhalla, Photon, Caddy)
make dev-up

# Detener servicios
make dev-down

# Validar y compilar backend Rust
make backend-check

# Compilar aplicación Android (modo debug)
make android-build

# Ejecutar pipeline de procesamiento de datos OSM
make data-build
```

## 4. Licencia y Atribución

- Código: [GNU Affero General Public License v3.0 or later (AGPL-3.0-or-later)](LICENSE).
- Datos cartográficos: © [OpenStreetMap contributors](https://www.openstreetmap.org/copyright) bajo licencia [ODbL](https://opendatacommons.org/licenses/odbl/). Ver [ATTRIBUTION.md](ATTRIBUTION.md).
- Política de contribuciones y CLA: Ver [CONTRIBUTING.md](CONTRIBUTING.md).
