<!--
SPDX-FileCopyrightText: 2026 Gabriel Piñones
SPDX-License-Identifier: AGPL-3.0-or-later
-->

# ADR 0003: Almacenamiento Estático y Descarga Local de PMTiles

## Estado
Aceptado. El punto 2 se precisó al implementar el pipeline de datos (`data/README.md`): el estilo no declara sprites ni glifos mientras no tenga capas de símbolos, y se publica un manifiesto con el tamaño y el sha256 del mapa.

## Contexto
Las aplicaciones de navegación tradicionales consultan servidores de tiles dinámicos o APIs comerciales (Mapbox, Google Maps, Stadia Maps) mediante HTTP por cada tesela visualizada. Esto presenta serias desventajas para ciclistas:
1. Las zonas con baja cobertura celular dejan el mapa en blanco.
2. El tráfico de red constante agota la batería y el plan de datos móviles del usuario.
3. El protocolo PMTiles remoto permite consultar teselas individuales por rango de bytes HTTP (Range Requests), pero los SDKs móviles nativos de MapLibre carecen de un sistema de caché de disco confiable y determinista para Range Requests HTTP sin conexión.

## Decisión
1. Generar los mapas vectoriales usando **Planetiler** en formato archivo único **PMTiles** (`.pmtiles`).
2. Publicar los archivos `.pmtiles` y los estilos visuales (`style.json`; sprites y glifos cuando el estilo tenga capas de símbolos) como archivos estáticos servidos por Caddy o almacenamiento de objetos compatible con S3, junto con un `manifest.json` (tamaño y sha256) para que la app verifique la descarga.
3. La aplicación móvil ofrece descargar el archivo regional de mapa en el almacenamiento local del dispositivo.
4. El visor MapLibre Native Android carga el mapa directamente desde el sistema de archivos local utilizando el protocolo `pmtiles://file:///...`.

## Consecuencias
### Positivas
- Disponibilidad 100% offline del mapa base completo, incluso sin señal celular.
- Cero costo recurrente por peticiones de teselas (no hay servidores de tiles dinámicos como TileServer GL corriendo permanentemente).
- Renderizado vectorial extremadamente fluido con bajo consumo de batería.

### Negativas / Riesgos
- El usuario debe descargar previamente el archivo de su región (típicamente entre 50 MB y 300 MB según la zona).
- Las actualizaciones del mapa base requieren descargas periódicas de archivos nuevos.
