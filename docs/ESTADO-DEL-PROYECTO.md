<!--
SPDX-FileCopyrightText: 2026 Gabriel Piñones
SPDX-License-Identifier: AGPL-3.0-or-later
-->

# Estado del proyecto Baze — traspaso

Documento de traspaso: **qué se hizo, qué falta y cómo retomar** el trabajo sin depender de la conversación que lo produjo. Se actualiza al cerrar cada etapa (ver [Registro de avance](#6-registro-de-avance-de-la-fase-actual)). La fuente canónica de reglas del repo sigue siendo [AGENTS.md](../AGENTS.md).

- **Rama de trabajo**: `FirstVersion` (no existe ningún Pull Request; no se abre ninguno sin petición expresa).
- **Origen**: auditoría de seguridad y funcionalidad (19 hallazgos: SEC-01…12, FUN-01…07) y plan de remediación por rondas.
- **Última actualización**: ver la fecha del último commit que toca este archivo (`git log -1 -- docs/ESTADO-DEL-PROYECTO.md`).

---

## 1. Resumen en diez líneas

1. La **Ronda 1** (seguridad, persistencia real en PostGIS, infraestructura endurecida, CI verde, Android compilando, documentación) está **implementada, empujada y verificada en el CI real**.
2. El backend ya **no es un stub en lo que toca a datos**: cuentas, reportes y votos viven en PostgreSQL/PostGIS, con migraciones, rol de BD sin privilegios, purga de caducados, topes anti-abuso y consultas de corredor.
3. **Ruteo y geocodificación siguen respondiendo `501`** a propósito (fail-closed): devolver una ruta falsa es peor que fallar. Se retiran en las Etapas A y B.
4. La app Android **compila y pasa lint, pruebas y R8**, pero sigue siendo un andamiaje: no es funcional todavía (Etapa C).
5. Las imágenes de motores del `compose.yaml` (`gisops/valhalla:3.4.0`, `ghcr.io/komoot/photon:0.5.0`) **no existen** tal cual; el perfil `engines` no puede arrancar hasta la Etapa B.
6. Fase en curso autorizada por el mantenedor: **Etapas 0 + A + B**. Las Etapas C y D están planificadas pero **no autorizadas**.

---

## 2. Reglas de entrega (decisiones del mantenedor)

| Tema | Regla |
|---|---|
| Ramas | Todo en `FirstVersion`, empujado a esa rama. Sin otras ramas ni PRs salvo petición. **Nunca `force push`**. No tocar la configuración git del repo. |
| Commits | Grupos de commits convencionales (`feat(...)`, `fix(...)`, `test(...)`, `docs(...)`, `ci(...)`…), uno por unidad lógica. Mensajes en inglés. |
| Identidad | Autor y committer `gabriel <131714613+gabrielpinones@users.noreply.github.com>` y trailer `Co-authored-by: kripiman <119073556+kripiman@users.noreply.github.com>`. **Sin atribución a Claude** en commits ni PRs ("no te adjuntes méritos"). El reparto autor/co-autor entre los dos correos es una suposición pendiente de confirmar. |
| Idioma | Código, comentarios y mensajes de commit en inglés; documentación en español. Cabecera SPDX (`AGPL-3.0-or-later`) en archivos nuevos. |
| Votación | ADR-0006 + edad de cuenta y topes (ADR-0007): dedupe por /24 (IPv4) y /64 (IPv6), el voto del creador cuenta como el primero; cuenta ≥ 600 s para reportar/votar; 30 reportes y 200 votos por día. Subredes en BD como HMAC con clave (ADR-0008). |
| Ruteo | **Fail-closed**: una ruta que cruce un cierre confirmado nunca se devuelve como válida. |

Script auxiliar para commitear con esa identidad (hay que recrearlo si el contenedor se reinicia; no está en el repo):

```bash
#!/usr/bin/env bash
# Uso: gc.sh "<subject>" ["<body>"]   (hacer `git add` antes)
set -euo pipefail
subject="$1"; body="${2:-}"
msg="$subject"; [ -n "$body" ] && msg="$msg

$body"
msg="$msg

Co-authored-by: kripiman <119073556+kripiman@users.noreply.github.com>"
GIT_AUTHOR_NAME="gabriel" GIT_AUTHOR_EMAIL="131714613+gabrielpinones@users.noreply.github.com" \
GIT_COMMITTER_NAME="gabriel" GIT_COMMITTER_EMAIL="131714613+gabrielpinones@users.noreply.github.com" \
  git commit -q -m "$msg"
```

---

## 3. Qué se hizo (Ronda 1)

### 3.1 Hallazgos de la auditoría

| ID | Tema | Estado | Dónde |
|---|---|---|---|
| SEC-01 | Secreto JWT público aceptado en producción | **Hecho** | `26e6b31`, `c8c599b`; tests que leen `.env.example` |
| SEC-02 | Rate limit evadible | **Hecho** | `c19ab3e`: clave por red del cliente (IPv4 /32, IPv6 /64), memoria acotada |
| SEC-03 | Votación manipulable, tipo elegido por el cliente | **Hecho** | `1a59b87`, `77298f4`, `5211499`; ADR-0006/0007; tipo derivado de la categoría en el servidor |
| SEC-04 | Identificador de cuenta público y datos en logs | **Hecho** | DTO público sin `creator_account_id`; Caddy sin logs de peticiones (`84d832f`) |
| SEC-05 | Agotamiento de recursos | **Hecho** | `ee653d1`: plazos, tope de concurrencia, cuerpo ≤ 16 KiB, SSE acotado; paginación; purga |
| SEC-06 | Tokens sin expiración ni forma canónica | **Hecho** | `513a47d`: tokens v2 `baze_v2.<payload>.<hmac>`, rotación con `JWT_SECRET_PREVIOUS` |
| SEC-07 | Caddy con listado de directorios | **Hecho** | `23e23f0` |
| SEC-08 | Contenedores sin endurecer; app como superusuario de BD | **Hecho** | `23e23f0`, `bde9fa6` (rol `baze_app` sin DDL), `5181221` |
| SEC-09 | Configuración sin validar; `/source` sin hash real | **Hecho** | `26e6b31`, `5181221` (commit incrustado en la imagen) |
| SEC-10 | Docs de API en producción; build no hermético | **Hecho** | `24e3a79` (swagger-ui vendorizado, reqwest sin TLS) |
| SEC-11 | Validación de entrada y errores | **Hecho** | `f4ecff7` (descripción por caracteres, sin control/bidi), `4d5f31d` (sin detalles internos, `error_id`) |
| SEC-12 | Dependencias Android antiguas | **Parcial** | Hay Dependabot (cargo, actions, gradle, docker). **No** se subieron versiones ni hay `verification-metadata.xml` de Gradle |
| FUN-01 | Núcleo del backend son stubs | **Parcial** | Hecho: PostGIS, cuentas, reportes, votos, corredor, purga (`77298f4`, `97ffe40`, `513a47d`, `8b4b59e`). **Falta**: cliente Valhalla y Photon (Etapas A/B) |
| FUN-02 | CI roto o no determinista | **Hecho** | `799661d`, `f86f11b`, `24f29f5`, `2ac2760`, `c7811cf`, `d9b7e7e`, `95040c9`, `4331c33`, `6c35ef1` |
| FUN-03 | Android no funcional | **Parcial** | Hecho: build verde, `namespace`, convention plugins, R8, `checkPurity`, licencias. **Falta** todo lo funcional (Etapa C, no autorizada) |
| FUN-04 | Contrato OpenAPI y modelos incompatibles | **Parcial** | Hecho: contrato regenerado y verificado en CI, modelos Android alineados y probados (`144dd18`). **Falta** SSE documentado con `resync` (Etapa 0) y cliente Android sobre fixtures dorados (Etapa C) |
| FUN-05 | Pipeline de datos con errores probables | **Pendiente** | Etapa B |
| FUN-06 | Cobertura de pruebas casi nula | **Hecho** | 134 tests sin BD y 196 con PostGIS (última corrida local), regresión e2e contra el binario real |
| FUN-07 | Gobernanza y documentación | **Parcial** | Hecho: ADR-0007/0008, `docs/operations.md`, SECURITY, CONTRIBUTING, ATTRIBUTION, plantillas, CODEOWNERS. **Falta** decisiones del mantenedor (CLA, buzón de seguridad, PVR) |

### 3.2 Evidencia en el CI real

| Workflow | Resultado | Commit |
|---|---|---|
| Backend CI (fmt, clippy `-D warnings`, tests con servicio PostGIS, e2e `--through 10`, cargo-deny, MSRV 1.88, contrato OpenAPI) | Verde | `17c0087` |
| Infrastructure CI (compose, Caddy, pila real, invariantes, `baze_app` sin DDL) | Verde | `2b182b2` |
| Android CI (lint, tests, `checkPurity`, `assembleDebug`, `assembleRelease` con R8, `licenseeAndroidRelease` + `checkLicenses`) | Verde, run 37221895535 | `0bae8d0` |
| Secrets Audit (gitleaks) | Verde | `a7dbcff` |

`FirstVersion` == `origin/FirstVersion` (54 commits sobre `Initial commit`).

### 3.3 Qué contiene hoy el backend

- Workspace Rust (edition 2024, MSRV 1.88, toolchain 1.97.1) con crates `app`, `shared`, `auth`, `hazards`, `routing`, `geocoding`, `realtime`. Los crates de dominio solo dependen de `shared`.
- `hazards`: servicio sobre PostGIS (`HazardStore`, `CorridorHazards`), votación en una transacción con `SELECT … FOR UPDATE`, índice único parcial `(hazard_id, voter_net) WHERE counts`, resolución terminal por `down - up >= umbral`, consultas de corredor con `ST_DWithin(geography)` e índice GiST, polígonos de bloqueo (buffer de 30 m), más de 500 cierres ⇒ `Unavailable` (fail-closed), purga cada 5 min y al arrancar.
- `auth`: cuentas persistidas, tokens v2 con rotación de clave.
- `app`: router Axum, límites (cuerpo, plazo, concurrencia, rate limit acotado), SSE acotado (25 por cliente, 8192 global, 30 min), `/health`, `/source`, `export-openapi`, `migrate`, `healthcheck`.
- Migraciones SQLx verificadas por checksum; el servidor se niega a arrancar si una migración aplicada cambió.

---

## 4. Qué falta

### 4.1 Fase en curso — autorizada por el mantenedor (Etapas 0 + A + B)

**Etapa 0 — Cierre de la Ronda 1**
- SSE: hoy `realtime` descarta en silencio los eventos perdidos (`BroadcastStreamRecvError::Lagged ⇒ None`). Pasar a `StreamEvent { Hazard, Resync }` y emitir `event: resync`; documentarlo en OpenAPI y regenerar `contracts/openapi.json`; pruebas de unidad y HTTP.
- `AGENTS.md` desactualizado (falta `infra.yml`/`security.yml` en la lista de workflows y la descripción actual del job Android).
- Aviso cosmético de licensee: `Allowed SPDX identifier 'BSD-3-Clause' is unused`.

**Etapa A — Ruteo y búsqueda reales (backend)**
- Cliente Valhalla en `routing` (`valhalla.rs`, `polyline.rs`, `exclusions.rs`): `POST /route`, `costing: bicycle`, decodificador polyline6, maniobras, ascenso/descenso desde `elevation`; errores ⇒ `Upstream` (502 genérico) o `NotFound`; **nunca registrar coordenadas** (ADR-0005).
- Evitación de cierres (ADR-0002): ruta base ⇒ consulta de corredor (15 m) ⇒ si hay cierres, nueva petición con `exclude_polygons`; máximo 3 rondas; más de 50 polígonos ⇒ 503; sin alternativa ⇒ 404.
- `shared::simplify_line` (Douglas-Peucker) para respetar `MAX_CORRIDOR_POINTS = 10 000` y límite de distancia ≤ 150 km en `RouteRequest::validate`.
- Cliente Photon en `geocoding` (`GET /api` con `.query()`, nunca interpolar `q`; no registrar el texto buscado).
- Pruebas con Valhalla/Photon falsos en proceso, integración con BD, e2e con un Valhalla de juguete en `scripts/e2e/audit_regression.py`.
- **El `501` se mantiene en el despliegue hasta el final de B.** Parada para revisión al terminar A.

**Etapa B — Pipeline de datos y motores**
- Imagen de Valhalla oficial `ghcr.io/valhalla/valhalla` fijada por digest; Photon con imagen propia (JRE fijado por digest, JAR 1.x con sha256 verificado, usuario no root); Planetiler 0.10.x.
- Scripts `data/scripts/01..05` endurecidos (checksums, `--proto '=https'`, elevación **obligatoria**, extracciones atómicas, publicación de `tiles.pmtiles.sha256`).
- `data/styles/style.json` con marcador `{{PMTILES_URL}}`, atribución y sin sprite/glyphs que no existen.
- `.github/workflows/data-smoke.yml` (manual y semanal): extracto de Mónaco por los scripts, motores + backend, ruta real con ascenso, ruta que cambia con un cierre confirmado, búsqueda real con un fixture de Photon versionado.
- Caddy: `Cache-Control`/`Accept-Ranges` para `/static/tiles.pmtiles`; docs de motores y datos.
- **El `501` solo se retira si `data-smoke.yml` pasa.**

### 4.2 Planificado, NO autorizado (requiere nueva aprobación del mantenedor)

- **Etapa C — Android funcional**: cliente Ktor real (cuentas, reportes, votos, ruta, SSE con reconexión y `resync`), almacenamiento del token con Android Keystore, permisos en tiempo de ejecución y servicio en primer plano correcto, MapLibre ≥ 11.8 con PMTiles (el catálogo fija 11.7.0, que no lo soporta), estilo en tiempo de ejecución, descarga verificada del mapa, selector de categoría, pantalla "Acerca de". Solo verificable en el CI de Android (~6,5 min por corrida).
- **Etapa D — Documentación de cierre**: ADR-0009 (mapas offline y almacenamiento del token), ADR de despliegue de motores y procedencia de datos, notas de MapLibre en ADR-0003. (Lo que dependa de A/B se documenta dentro de B.)

### 4.3 Deuda conocida fuera de las etapas

- SEC-12: versiones de Android sin actualizar y sin verificación de dependencias de Gradle.
- Riesgo V7: el permiso `pull-requests: read` de gitleaks se añadió de forma preventiva; no se ha podido verificar porque no hay PRs.
- La auditoría original dice 11 tests; los reales eran 10 (dato menor).

### 4.4 Decisiones del mantenedor (no bloquean las Etapas 0 y A)

1. Reparto autor/co-autor de los dos correos (hoy: autor `gabrielpinones`, co-autor `kripiman`).
2. CLA: redactar el texto o mantener la política interina de `CONTRIBUTING.md` (sin PRs de código externos).
3. Crear el buzón `security@baze.cl` y activar Private Vulnerability Reporting.
4. Región del primer despliegue (el script usa Chile) y si GraphHopper publica un índice de Photon para ella.
5. Ancho de banda de teselas: CDN, almacenamiento externo, o aceptar que Caddy sirva `tiles.pmtiles`.
6. Dominio público y fecha de despliegue (la app de release exigirá `baze.apiBaseUrl`).
7. ¿Abrir un Pull Request de `FirstVersion` hacia `main`?

---

## 5. Cómo retomar

### 5.1 Entorno del sandbox

```bash
# El clúster local no arranca solo si el contenedor se reinició:
pg_ctlcluster 16 main start
export DATABASE_URL=postgres://postgres@127.0.0.1:5432/postgres   # trust auth, solo local; usar con bases desechables
```

### 5.2 Compuerta local antes de cada push

```bash
make backend-check        # fmt, clippy -D warnings, tests, export-openapi --check
make backend-db-test      # incluye las pruebas con PostGIS (necesita DATABASE_URL)
(cd backend && cargo deny check bans licenses sources && cargo +1.88.0 check --locked --workspace)
bash scripts/check-toolchain-sync.sh && bash scripts/compose-check.sh
shellcheck scripts/*.sh scripts/e2e/*.sh data/scripts/*.sh
url="$(bash scripts/e2e/prepare-db.sh "$DATABASE_URL")"
python3 scripts/e2e/audit_regression.py --database-url "$url" --admin-database-url "$DATABASE_URL" --through <N>
```

Tras cada push se revisan los workflows (`backend.yml`, `infra.yml`, `android.yml`, `secrets.yml`, `security.yml`) con las herramientas MCP de GitHub y se corrige antes de seguir. Una etapa se cierra solo con CI verde.

### 5.3 Qué no se puede verificar desde el sandbox

| Limitación | Cómo se cubre |
|---|---|
| Sin daemon de Docker | `infra.yml` y `data-smoke.yml` en el CI |
| Sin Android SDK ni Google Maven | `android.yml` (iterar contra el CI real) |
| `download1.graphhopper.com` bloqueado | Fixture de Photon en el smoke; el índice de Chile lo confirma el mantenedor |
| Decisiones legales y de política | Mantenedor (4.4) |

Sí se puede consultar desde el sandbox: GitHub (`git ls-remote`, assets de releases), Maven Central, plugins.gradle.org, la distribución de Gradle y las APIs de Docker Hub y `ghcr.io` (tags y digests).

### 5.4 Reglas del repo que no hay que romper (resumen de AGENTS.md)

- Crates de dominio (`auth`, `hazards`, `routing`, `geocoding`, `realtime`) dependen solo de `shared`; el cableado va en `app`.
- Handlers delgados; GeoJSON en HTTP; configuración solo por variables de entorno validadas.
- Sin Redis/Kafka/microservicios; sin imágenes `latest`; sin secretos en el repo; sin trazas GPS ni ubicaciones puntuales (ADR-0005).
- `core/model` y `core/domain` de Android son Kotlin puro (lo verifica `checkPurity`).
- Las migraciones aplicadas nunca se editan: cambios en un archivo nuevo.

### 5.5 Hechos verificados sobre los motores (para la Etapa B)

- `gisops/valhalla:3.4.0` no existe. La imagen oficial es `ghcr.io/valhalla/valhalla` (tags hasta 3.9.0; el digest se resuelve con la API del registro).
- `ghcr.io/komoot/photon:0.5.0` no parece existir. Photon se distribuye como JAR (tags hasta 1.3.0, basado en OpenSearch, formato de BD `1.0`); los índices los publica GraphHopper.
- Planetiler: el script fija 0.8.2; la última es 0.10.2.
- Valhalla (docs 3.6.0): `exclude_polygons` son anillos anidados `[lon,lat]`; `elevation_interval` devuelve `elevation` por tramo si los datos se generaron con elevación; hay un límite de perímetro total de exclusiones (confirmar el valor por defecto en la imagen elegida).
- MapLibre Native Android: PMTiles llegó en 11.8.0; el catálogo fija 11.7.0; última 13.6.1.
- El contrato de maniobras (`instruction`, `distance_meters`, `time_seconds`, `location`) coincide entre backend y Android.

---

## 6. Registro de avance de la fase actual

| Etapa | Estado | Commits | Evidencia de CI |
|---|---|---|---|
| Handoff (este documento) | En curso | — | — |
| 0 — resync SSE, AGENTS.md, licensee | Pendiente | — | — |
| A — clientes Valhalla y Photon | Pendiente | — | — |
| B — pipeline de datos y smoke | Pendiente | — | — |
| C — Android funcional | No autorizada | — | — |
| D — documentación de cierre | No autorizada | — | — |
