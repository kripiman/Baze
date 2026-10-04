<!--
SPDX-FileCopyrightText: 2026 Gabriel Piñones
SPDX-License-Identifier: AGPL-3.0-or-later
-->

# ADR 0002: Re-ruteo Condicionado Exclusivamente a Bloqueos Confirmados

## Estado
Aceptado. Enmendado por [ADR-0007](0007-identity-voting-and-abuse-limits.md): un reporte también puede ser retirado por la comunidad (`resolved`).

## Contexto
En una aplicación de navegación abierta, los reportes comunitarios presentan diferentes grados de severidad y certeza. Un reporte erróneo, malicioso o preliminar (ej. un usuario reportando una calle cortada por error) no debe penalizar drásticamente la ruta de todos los ciclistas ni causar desvíos innecesarios. Además, incidentes como "vidrio en la calzada" o "bache" son advertencias (`warning`) que demandan precaución pero no impiden el paso ni justifican desviar al usuario kilómetros alrededor.
Por otro lado, el motor de ruteo Valhalla limita la cantidad y perímetro de polígonos que pueden excluirse en la solicitud (`exclude_polygons`), por lo que enviar todos los reportes de una ciudad degradaría el rendimiento o provocaría errores `400 Bad Request`.

## Decisión
1. **Diferenciación estricta de severidad**:
   - `warning`: solo genera alertas de proximidad en la app móvil. Nunca modifica la ruta.
   - `blocking`: representa interrupciones transitables (calles cerradas, obras, inundaciones).
2. **Umbral de confirmación**:
   - Un reporte `blocking` recién creado nace en estado `unconfirmed`.
   - Solo cuando la suma neta de votos (`upvotes - downvotes`) alcanza el umbral configurable (`HAZARD_CONFIRMATION_THRESHOLD`, por defecto 3), el estado pasa a `confirmed`.
3. **Cruce espacial antes de invocar re-ruteo**:
   - El backend solicita la ruta óptima a Valhalla.
   - Si la ruta no cruza ningún bloqueo confirmado en PostGIS, se retorna de inmediato.
   - Solo si existe intersección espacial con la geometría de la ruta, se extraen dichos bloqueos y se genera un conjunto reducido de polígonos para re-invocar Valhalla con `exclude_polygons`.

## Consecuencias
### Positivas
- Se evitan desvíos absurdos causados por reportes no confirmados o advertencias leves.
- Se minimiza el uso de CPU y memoria en Valhalla respetando los límites de `exclude_polygons`.
- La comunidad adquiere un rol activo de validación mediante votación.

### Negativas / Riesgos
- Si un bloqueo real aún no tiene suficientes votos, la primera persona que transite por ahí recibirá la advertencia pero la ruta no la evitará automáticamente.
