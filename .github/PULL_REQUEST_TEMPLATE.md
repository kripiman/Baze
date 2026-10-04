<!--
SPDX-FileCopyrightText: 2026 Gabriel Piñones
SPDX-License-Identifier: AGPL-3.0-or-later
-->

## Qué cambia y por qué

<!-- El problema que resuelve y las decisiones tomadas. Enlaza el issue o el ADR si existe. -->

## Cómo se probó

<!-- Pruebas añadidas o ejecutadas. Para cambios de comportamiento, la prueba que falla sin este cambio. -->

## Lista de verificación

- [ ] Los commits siguen Conventional Commits, en inglés (ver `CONTRIBUTING.md`).
- [ ] `make backend-check` pasa (y `make backend-db-test` si toca SQL o `hazards`/`auth`).
- [ ] Si cambia la API, `contracts/openapi.json` está regenerado (`make openapi`).
- [ ] Si cambia la base de datos, hay una migración **nueva**; ninguna migración aplicada fue editada.
- [ ] Si toca `data/`, `infra/photon/` o los clientes de ruteo y búsqueda, el workflow `data-smoke.yml` pasa.
- [ ] Si toca Android, `make android-build` pasa y los módulos `core` siguen sin importar `java.*` ni `android.*`.
- [ ] No incluye secretos, tokens ni archivos `.env`.
- [ ] Si cambia una decisión de diseño, hay un ADR nuevo o una enmienda.
- [ ] Soy colaborador del proyecto o el CLA está firmado (ver `CONTRIBUTING.md`, sección 2).
