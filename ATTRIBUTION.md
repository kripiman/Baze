<!--
SPDX-FileCopyrightText: 2026 Gabriel Piñones
SPDX-License-Identifier: AGPL-3.0-or-later
-->

# Atribuciones y Licencias de Terceros

Este proyecto utiliza y distribuye datos y software de terceros bajo las siguientes licencias y condiciones de atribución.

## 1. Datos de Mapas y Ruteo

- **OpenStreetMap**: Los datos cartográficos, redes viales y puntos de interés provienen de [OpenStreetMap](https://www.openstreetmap.org/copyright), © Colaboradores de OpenStreetMap, distribuidos bajo la licencia [Open Database License (ODbL)](https://opendatacommons.org/licenses/odbl/).
- **Esquema de Tiles (OpenMapTiles / Planetiler)**: Los vector tiles generados mediante Planetiler utilizan la especificación de OpenMapTiles, © [OpenMapTiles](https://openmaptiles.org/).

### Requisitos de Atribución en la Aplicación
1. **Vista de Mapa Principal**: La esquina inferior del mapa interactivo de la app móvil incluye de forma visible:
   `© OpenStreetMap contributors` con enlace a `https://www.openstreetmap.org/copyright`.
2. **Sección "Acerca de"**:
   - Enlace directo a los términos de licencia ODbL y OpenStreetMap.
   - Enlace al repositorio de código fuente abierto según la sección 13 de GNU AGPLv3.
   - Lista completa de librerías y dependencias de código abierto con sus licencias correspondientes.

## 2. Componentes de Software Abierto

- **MapLibre Native Android**: Licenciado bajo BSD-2-Clause / Apache-2.0.
- **Valhalla Routing Engine**: Licenciado bajo MIT.
- **Photon Geocoder**: Licenciado bajo Apache-2.0.
- **Planetiler**: Licenciado bajo Apache-2.0.
- **Caddy Web Server**: Licenciado bajo Apache-2.0.
- **PostGIS / PostgreSQL**: Licenciados bajo GPLv2+ y PostgreSQL License.

## 3. Dependencias de Código

Las dependencias directas y transitivas se auditan automáticamente en el CI, de modo que ninguna con una licencia no aprobada llega a una versión distribuible:

- **Backend (Rust)**: `cargo deny` aplica una lista de licencias permitidas (AGPL-3.0-or-later, MIT, Apache-2.0, BSD-2-Clause, BSD-3-Clause, ISC, MPL-2.0, Unicode-3.0, Unicode-DFS-2016 y Zlib) y vigila avisos de seguridad (ver `backend/deny.toml`). Entre las bibliotecas principales: Axum, Tokio, Tower, Hyper, SQLx, serde, chrono y uuid (MIT y/o Apache-2.0), utoipa (MIT o Apache-2.0) y Swagger UI (Apache-2.0, solo se sirve cuando `ENABLE_API_DOCS` está activo).
- **App Android (Kotlin)**: el plugin *licensee* falla la compilación (`./gradlew checkLicenses`) si el APK *release* incluye una dependencia fuera de Apache-2.0, MIT, BSD-2-Clause o BSD-3-Clause. Entre las bibliotecas principales: Kotlin, kotlinx.coroutines, kotlinx.serialization y kotlinx-datetime (Apache-2.0), Jetpack Compose y AndroidX (Apache-2.0), Ktor (Apache-2.0), Koin (Apache-2.0) y MapLibre Native (BSD-2-Clause).

La sección "Acerca de" de la app, que listará estas dependencias con sus licencias, está pendiente de implementar (ver "Requisitos de Atribución en la Aplicación").

## 4. Datos de Terceros en el Pipeline

- **Extractos de OpenStreetMap**: descargados de [Geofabrik](https://download.geofabrik.de/), bajo ODbL (ver sección 1).
- **Elevación (SRTM)**: usada por Valhalla para las pendientes de las rutas ciclistas (ver `data/scripts/03-build-valhalla.sh`). Antes de distribuir artefactos derivados, confirma las condiciones de uso del conjunto exacto que se descargue. <!-- TODO(verify): fijar la fuente y la licencia exactas del modelo de elevación -->
