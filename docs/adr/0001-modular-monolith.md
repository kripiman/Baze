<!--
SPDX-FileCopyrightText: 2026 Gabriel Piñones
SPDX-License-Identifier: AGPL-3.0-or-later
-->

# ADR 0001: Monolito Modular en Backend

## Estado
Aceptado

## Contexto
El sistema requiere gestionar autenticación anónima, geocodificación, persistencia espacial de reportes comunitarios, orquestación de ruteo ciclista y difusión de eventos en tiempo real. En proyectos modernos suele plantearse una separación temprana en microservicios, apoyados por brokers como Kafka o bases en memoria como Redis. Sin embargo, para la etapa inicial y MVP de Baze, la sobrecarga operativa, la latencia entre servicios, la dificultad de observabilidad y los costos de infraestructura en un solo VPS no justifican una arquitectura distribuida.

## Decisión
Implementar el backend como un **monolito modular en Rust** dentro de un único proceso compilado y un solo workspace de Cargo (`backend/crates/`):
- Los módulos de dominio (`auth`, `hazards`, `routing`, `geocoding`, `realtime`) se aíslan en crates independientes.
- Todos los crates de dominio dependen únicamente de `shared` y de librerías externas esenciales.
- Ningún crate de dominio depende de otro crate de dominio.
- La comunicación o integración entre módulos se realiza mediante traits abstractos definidos en `shared` e inyectados en tiempo de inicialización por el crate binario `app`.
- La mensajería de eventos en tiempo real (SSE) se gestiona en memoria mediante canales Tokio `broadcast`. En caso de escalamiento horizontal futuro a múltiples réplicas, se adoptará `LISTEN/NOTIFY` nativo de PostgreSQL antes de introducir infraestructura externa como Redis.

## Consecuencias
### Positivas
- Despliegue extremadamente simple: un único contenedor Docker ligero (~20MB imagen final).
- Tiempo de compilación optimizado y reutilización de caché por crate en Rust.
- Sin costo de serialización interna de red (RPC/HTTP/gRPC) entre módulos.
- Base de código limpia con límites de dominio explícitos y comprobados por el compilador de Rust.

### Negativas / Riesgos
- El escalamiento horizontal de réplicas requerirá migrar el SSE de memoria local a `LISTEN/NOTIFY` en PostgreSQL.
