<!--
SPDX-FileCopyrightText: 2026 Gabriel Piñones
SPDX-License-Identifier: AGPL-3.0-or-later
-->

# ADR 0005: Exclusión Total de Telemetría y Trazas GPS en el MVP

## Estado
Aceptado

## Contexto
Aplicaciones tradicionales de navegación como Waze o Google Maps envían continuamente la posición GPS de los usuarios al backend para estimar velocidades de tráfico, tiempos de viaje en vivo y mapas de calor.
En el contexto de ciclistas urbanos y de Baze:
1. Las trazas continuas de geolocalización constituyen datos extremadamente sensibles y revelan hábitos de vida, domicilios y lugares de trabajo.
2. Almacenar o procesar flujos de telemetría masiva requeriría una infraestructura compleja de ingesta en tiempo real (brokers como Kafka, almacenes de series temporales, worker pools), inviable e innecesaria para un MVP alojado en un único VPS.
3. El cálculo de proximidad a peligros viales puede ejecutarse enteramente en el dispositivo del usuario.

## Decisión
1. **Cero recolección de telemetría**: La app Baze nunca envía coordenadas GPS en segundo plano ni registra trazas de trayectos en el servidor.
2. **Consultas de ruteo anónimas y no persistidas**: Las solicitudes de cálculo de ruta solo envían coordenadas de origen y destino puntuales; el servidor las procesa en memoria y no asocia ni registra historiales de trayectos a las cuentas.
3. **Cálculo local de proximidad**: La app recibe la lista de peligros a lo largo del corredor de la ruta y evalúa la proximidad en el dispositivo usando el `LocationManager` de Android.
4. **Suscripción SSE por bounding box**: La app se suscribe a eventos en tiempo real indicando un polígono o bounding box general del área, sin transmitir la posición exacta en movimiento.

## Consecuencias
### Positivas
- Máximo respeto a la privacidad del usuario y total confianza comunitaria.
- Reducción drástica del ancho de banda y del consumo de batería en el móvil.
- Infraestructura del backend sumamente ligera y libre de bases de datos de series temporales o colas de eventos.

### Negativas / Riesgos
- No se dispone de estimación de congestión o velocidades promedio basadas en el tráfico ciclista en vivo (aunque el ruteo ciclista se beneficia mucho más de la infraestructura y pendientes que del tráfico automotor denso).
