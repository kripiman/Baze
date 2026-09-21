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
