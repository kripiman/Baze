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
Co-authored-by: kripiman <119073556+kripiman@users.noreply.github.com>
```

## 2. Acuerdo de Licencia de Colaborador (CLA)

Para proteger la capacidad de publicar la aplicación en tiendas que imponen términos restrictivos incompatibles con copyleft estricto (como Apple App Store para la versión futura de iOS), todos los colaboradores externos deben aceptar un **Acuerdo de Licencia de Colaborador (Contributor License Agreement - CLA)** antes de que sus Pull Requests sean aceptados.

**Política interina**: el texto del CLA y su automatización todavía no existen, así que por ahora **no se aceptan Pull Requests de código de personas ajenas al proyecto**. Sí son bienvenidos los issues, los reportes de fallos, las propuestas de diseño y los reportes de seguridad (ver [SECURITY.md](SECURITY.md)). Esta política se levantará cuando el CLA esté redactado y activado. <!-- TODO(verify): el mantenedor debe redactar el CLA (o elegir otra política) y activar la automatización, p. ej. CLA Assistant -->

Para mayor detalle sobre las razones jurídicas y estratégicas de esta decisión, consulta [ADR-0004](docs/adr/0004-agpl-and-contributions.md).

## 3. Flujo de Trabajo (Pull Requests)

1. Haz un fork del repositorio y crea una rama descriptiva (`feat/hazard-voting-threshold` o `fix/pmtiles-cache`).
2. Agrega pruebas para cualquier cambio funcional.
3. Asegúrate de que las verificaciones locales pasen antes de abrir un PR:
   - Backend: `make backend-check` (formato, clippy sin advertencias, pruebas sin base de datos y contrato OpenAPI al día).
   - Backend con base de datos: `make backend-db-test` (ver abajo).
   - Android: `make android-build`. El CI además ejecuta lint, pruebas, `checkPurity` (los módulos `core` no importan `java.*` ni `android.*`), el ensamblado *release* con R8 y `checkLicenses`.
   - Licencias y avisos de seguridad de dependencias de Rust: `make backend-deny`.
   - Datos y motores (`data/`, `infra/photon/`, los clientes de `routing` y `geocoding`): `make style-check` y el workflow `data-smoke.yml`, que construye Mónaco con los scripts y prueba Valhalla y Photon reales a través del backend (se lanza solo al tocar esas rutas, o a mano desde la pestaña Actions).
   - Secretos: no incluir archivos `.env`, tokens ni claves privadas.
4. Abre un Pull Request describiendo el problema resuelto y las decisiones tomadas.

### Pruebas con base de datos

Las pruebas que usan PostgreSQL (`--features db-tests`, que activa `--all-features`) necesitan un servidor **desechable** con PostGIS y un superusuario, porque `sqlx::test` crea una base de datos nueva por cada prueba:

```bash
docker run --rm -d --name baze-test-db -p 127.0.0.1:5432:5432 \
  -e POSTGRES_PASSWORD=postgres postgis/postgis:16-3.4-alpine
DATABASE_URL=postgres://postgres:postgres@127.0.0.1:5432/postgres make backend-db-test
```

Nunca apuntes `DATABASE_URL` a una base de datos con datos que quieras conservar. El CI ejecuta las mismas pruebas contra un contenedor PostGIS.

Además, `scripts/e2e/audit_regression.py` reproduce contra el binario real los ataques que originaron las defensas de seguridad (ver su cabecera para el uso y `scripts/e2e/prepare-db.sh` para preparar la base de datos).

### Cambios en la base de datos

Las migraciones de `backend/migrations/` **nunca se editan una vez aplicadas**: el servidor se niega a arrancar si la suma de verificación de una migración aplicada cambió. Los cambios van en un archivo nuevo. Ver [ADR-0008](docs/adr/0008-database-roles-migrations-and-vote-networks.md).
