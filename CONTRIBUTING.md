<!--
SPDX-FileCopyrightText: 2026 Gabriel Piñones
SPDX-License-Identifier: AGPL-3.0-or-later
-->

# Guía de Contribución

Gracias por tu interés en contribuir a **Baze**. Para mantener la calidad del proyecto y asegurar la viabilidad técnica y legal del software, sigue estas pautas.

## 1. Convención de Commits y Lenguaje

- **Idioma**: Todo el código fuente, nombres de variables, identificadores, mensajes de error y mensajes de commit deben escribirse estrictamente en **inglés**. La documentación de usuario y arquitectura se mantiene en **español**.
- **Conventional Commits**: Todos los commits deben seguir el estándar [Conventional Commits v1.0.0](https://www.conventionalcommits.org/):
  - `feat: add anonymous token revocation`
  - `fix: correct bounding box calculation in route proximity`
  - `docs: update deployment instructions for Valhalla elevation`
  - `chore: update rust toolchain to edition 2024`
  - `refactor: extract GeoJSON polygon converter to shared crate`

### Atribución y Co-autoría
Si colaboras o generas commits asistidos, asegúrate de mantener la autoría adecuada o agregar coautores usando el estándar de Git:
```git
Co-authored-by: kripiman <131714613+gabrielpinones@users.noreply.github.com>
```

## 2. Acuerdo de Licencia de Colaborador (CLA)

Para proteger la capacidad de publicar la aplicación en tiendas que imponen términos restrictivos incompatibles con copyleft estricto (como Apple App Store para la versión futura de iOS), todos los colaboradores externos deben aceptar un **Acuerdo de Licencia de Colaborador (Contributor License Agreement - CLA)** antes de que sus Pull Requests sean aceptados.

<!-- TODO: Definir e integrar el texto completo del CLA y la automatización del bot (ej. CLA Assistant) -->

Para mayor detalle sobre las razones jurídicas y estratégicas de esta decisión, consulta [ADR-0004](docs/adr/0004-agpl-and-contributions.md).

## 3. Flujo de Trabajo (Pull Requests)

1. Haz un fork del repositorio y crea una rama descriptiva (`feat/hazard-voting-threshold` o `fix/pmtiles-cache`).
2. Agrega pruebas unitarias para cualquier cambio funcional.
3. Asegúrate de que las verificaciones locales pasen antes de abrir un PR:
   - Backend: `make backend-check`
   - Android: `make android-build`
   - Secretos: no incluir archivos `.env`, tokens ni claves privadas.
4. Abre un Pull Request describiendo el problema resuelto y las decisiones tomadas.
