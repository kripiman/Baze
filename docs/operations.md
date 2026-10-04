<!--
SPDX-FileCopyrightText: 2026 Gabriel Piñones
SPDX-License-Identifier: AGPL-3.0-or-later
-->

# Guía de Operación — Baze

Cómo configurar, desplegar y mantener el backend. Las decisiones de fondo están en los ADR [0007](adr/0007-identity-voting-and-abuse-limits.md) y [0008](adr/0008-database-roles-migrations-and-vote-networks.md).

> **Estado**: el backend persiste reportes, votos y cuentas. Ruteo y geocodificación tienen cliente real y los motores reales (Valhalla y Photon) se verifican sobre Mónaco en `data-smoke.yml`; con `ENGINES_ENABLED` apagado (el valor por defecto) responden `501 Not Implemented`. Antes de encender los motores en un despliegue público construye los datos de tu región y comprueba el resultado (sección 5.1).

## 1. Secretos y primer arranque

- `make dev-env` ejecuta `scripts/gen-env.sh`: crea `infra/.env` a partir de `infra/.env.example` con secretos aleatorios (modo 600, ignorado por git) y **nunca sobrescribe** uno existente. `make dev-up` lo invoca antes de levantar la pila.
- `infra/.env.example` no contiene ningún secreto. Los valores vacíos son obligatorios: Compose y el backend se niegan a arrancar mientras sigan vacíos.
- En producción genera cada secreto con `openssl rand -hex 32` y guárdalos en tu gestor de secretos; no los copies a un repositorio.
- Con `ENVIRONMENT=production` el backend **no arranca** si:
  - `JWT_SECRET` tiene menos de 32 bytes, contiene palabras de plantilla (`change_me`, `secret`, `password`, `example`…) o es de baja entropía;
  - `DATABASE_URL` usa el usuario `postgres`, el valor de ejemplo, o un rol con privilegios de más;
  - `GIT_COMMIT_HASH` no es el hash hexadecimal (7 a 40 caracteres) con el que se construyó la imagen (`/source` lo publica por la sección 13 de la AGPL);
  - cualquier número está fuera de rango o no es un número (el mensaje nombra la variable).
- **Despliegues anteriores**: si alguno arrancó con el `JWT_SECRET` de ejemplo que traía el repositorio (`change_me_to_a_random_32_bytes_secret_key_in_production`), cualquiera pudo fabricar tokens válidos. Debe rotarse (sección 3) y, como la autenticación anterior no se respaldaba en base de datos, tratar como sospechosas las cuentas y reportes creados.

## 2. Base de datos: roles y migraciones

- Dos roles (ADR-0008): el dueño del esquema (`POSTGRES_USER`) solo lo usa el servicio `migrate`; el backend usa `baze_app` (solo `SELECT/INSERT/UPDATE/DELETE`).
- `docker compose up` ejecuta `migrate` (un solo uso) y el backend espera a que termine bien. Para aplicarlas a mano: `baze-server migrate` con `DATABASE_URL` apuntando al rol dueño.
- El backend revisa al arrancar que todas las migraciones estén aplicadas y que ninguna haya sido alterada; si no, se niega a arrancar con un mensaje que lo dice.
- **Volúmenes de PostgreSQL anteriores al rol `baze_app`**: los scripts de `docker-entrypoint-initdb.d` solo corren con un volumen vacío. En uno existente, crea el rol ejecutando el mismo script dentro del contenedor y concede permisos sobre las tablas que ya existen (los permisos por defecto solo alcanzan a las que se creen después):

  ```bash
  docker compose -f infra/compose.yaml --env-file infra/.env exec -T postgis \
    bash /docker-entrypoint-initdb.d/01-roles.sh
  docker compose -f infra/compose.yaml --env-file infra/.env exec -T postgis sh -c \
    'psql -U "$POSTGRES_USER" -d "$POSTGRES_DB" -c "GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA public TO baze_app"'
  ```

  Ejecutar el script de nuevo también rota la contraseña de `baze_app` a la que tenga `APP_DB_PASSWORD`.

## 3. Rotación de `JWT_SECRET`

1. Copia el valor actual de `JWT_SECRET` a `JWT_SECRET_PREVIOUS`.
2. Pon el secreto nuevo (`openssl rand -hex 32`) en `JWT_SECRET` y reinicia el backend.
3. Los tokens ya emitidos siguen valiendo (se verifican con el anterior) hasta que caduquen (`AUTH_TOKEN_TTL_DAYS`, 180 por defecto); los nuevos se firman con el secreto nuevo.
4. Pasado ese plazo, vacía `JWT_SECRET_PREVIOUS`.

Efecto secundario: `JWT_SECRET` también es la clave de las etiquetas de red de voto (ADR-0008). Al rotarlo, una red puede volver a votar **una vez** en los reportes que sigan vivos (como máximo un TTL de reportes, 24 horas por defecto). Es esperado y acotado. Si el secreto se filtró, rota sin usar `JWT_SECRET_PREVIOUS`: se invalidan todos los tokens a la vez.

## 4. Moderación

Las cuentas son filas de `accounts`; los reportes, de `hazards`. Conéctate como el rol dueño:

```sql
-- Banear una cuenta: su token deja de valer en la siguiente petición (se consulta en cada una).
UPDATE accounts SET is_active = false WHERE id = '00000000-0000-0000-0000-000000000000';

-- Retirar un reporte (y, por cascada, sus votos).
DELETE FROM hazards WHERE id = '00000000-0000-0000-0000-000000000000';

-- Cuentas que más reportan en las últimas 24 horas.
SELECT creator_account_id, count(*) FROM hazards
WHERE created_at > now() - interval '24 hours'
GROUP BY 1 ORDER BY 2 DESC LIMIT 20;

-- Reportes de bloqueo confirmados que sigan vigentes.
SELECT id, category, upvotes, downvotes, created_at FROM hazards
WHERE hazard_type = 'blocking' AND status = 'confirmed' AND expires_at > now();
```

Por diseño la base de datos no guarda direcciones IP: no se puede saber desde qué red votó una cuenta, solo si dos votos comparten red mientras el reporte vive.

## 5. Configuración

Todas las variables se validan al arrancar. Vacío o ausente = valor por defecto.

| Variable | Defecto | Rango o valores | Qué controla |
|---|---|---|---|
| `ENVIRONMENT` | (obligatoria) | `development`, `production` | Activa las guardas de producción |
| `PORT`, `HOST` | `8080`, `0.0.0.0` | 1 a 65535 | Dirección de escucha |
| `DATABASE_URL` | (obligatoria) | URL de PostgreSQL | Rol `baze_app` del backend |
| `JWT_SECRET` | (obligatoria) | ≥ 32 bytes aleatorios en producción | Firma de tokens y etiquetas de red |
| `JWT_SECRET_PREVIOUS` | vacío | distinto de `JWT_SECRET` | Solo verifica, durante una rotación |
| `AUTH_TOKEN_TTL_DAYS` | `180` | 1 a 730 | Vigencia de los tokens |
| `RATE_LIMIT_REQUESTS_PER_MINUTE` | `60` | 1 a 10000 | Presupuesto por red de cliente |
| `HAZARD_CONFIRMATION_THRESHOLD` | `3` | 2 a 50 | Votos netos para confirmar o retirar |
| `HAZARD_DEFAULT_TTL_HOURS` | `24` | 1 a 8760 | Vida de un reporte |
| `ACCOUNT_MIN_AGE_SECONDS` | `600` | 0 a 86400 | Edad mínima para votar o reportar un bloqueo |
| `REPORTS_PER_ACCOUNT_PER_DAY` | `30` | 1 a 1000 | Tope diario de reportes por cuenta |
| `VOTES_PER_ACCOUNT_PER_DAY` | `200` | 1 a 5000 | Tope diario de votos por cuenta |
| `TRUSTED_PROXIES` | loopback y `172.16.0.0/12` (Compose: solo `172.30.0.2`) | IPs o CIDR separados por coma | Proxies a los que se cree `X-Real-IP` |
| `REQUEST_TIMEOUT_SECONDS` | `15` | 1 a 120 | Plazo por petición (408) |
| `MAX_CONCURRENT_REQUESTS` | `512` | 1 a 100000 | Peticiones simultáneas en vuelo |
| `ENABLE_API_DOCS` | `true` en development, `false` en production | `true`, `false` | Swagger UI y `/api-docs` |
| `ENGINES_ENABLED` | `false` | `true`, `false` | Usar Valhalla y Photon; apagado, ruteo y búsqueda responden 501 |
| `VALHALLA_URL`, `PHOTON_URL` | `http://localhost:8002`, `http://localhost:2322` | `http://` interno | Dónde están los motores (solo con `ENGINES_ENABLED=true`) |
| `ENGINES_UID`, `ENGINES_GID` | `1000`, `1000` (`make dev-env` pone los tuyos) | uid/gid numéricos | Usuario con el que corren los motores: el dueño de `data/out` |
| `VALHALLA_THREADS`, `PHOTON_JAVA_OPTS` | `2`, `-Xmx1g -Duser.home=/tmp` | número, opciones de JVM | Recursos de los motores (solo Compose) |
| `SOURCE_REPO_URL`, `GIT_COMMIT_HASH` | repositorio, `dev` | `https://`; hash hex en producción | Respuesta de `/source` |

Subir `ACCOUNT_MIN_AGE_SECONDS` o `HAZARD_CONFIRMATION_THRESHOLD` es la primera palanca si aparece abuso coordinado (ADR-0007, límites aceptados).

## 5.1. Motores y datos (Valhalla y Photon)

Ruteo y búsqueda de direcciones usan dos motores opcionales (perfil `engines` de Compose). Orden para activarlos:

1. **Construir los datos** (ver [`data/README.md`](../data/README.md)): `make data-build`, o los scripts 01 a 05 uno a uno. Photon necesita una fuente explícita (`PHOTON_IMPORT_FILE` o `PHOTON_DUMP_URL` con su sha256).
2. **Levantar los motores**: `docker compose -f infra/compose.yaml --env-file infra/.env --profile engines up -d --build`. Valhalla (imagen oficial fijada por digest) y Photon (imagen propia, jar verificado por checksum) corren **sin privilegios**, con el sistema de archivos en solo lectura, sin puertos publicados y solo en la red interna. `ENGINES_UID`/`ENGINES_GID` deben ser el usuario que construyó los datos.
3. **Encender el backend**: `ENGINES_ENABLED=true` en `infra/.env` y reiniciar `backend`. Con el valor apagado ruteo y búsqueda responden `501` y no se llama a ningún motor.
4. **Verificar**: `python3 scripts/e2e/engines_smoke.py --backend http://127.0.0.1:8080 --psql "…"` (ver su cabecera) comprueba, sobre el entorno de pruebas, ruta con elevación real, rodeo de un cierre confirmado y búsqueda. El workflow `data-smoke.yml` lo hace sobre Mónaco.

Memoria y recursos: Valhalla `mem_limit: 2g` y `VALHALLA_THREADS` (2 por defecto); Photon `mem_limit: 2g` con `PHOTON_JAVA_OPTS` (`-Xmx1g` por defecto; un índice nacional necesita más heap y más límite). Mantén el disco por debajo del 90 %: OpenSearch, que Photon lleva embebido, deja de asignar con el disco casi lleno.

**Actualizar los datos**: repite los pasos 01 a 05 con el mismo extracto para todos (nunca mezcles datos de fechas distintas), reinicia `valhalla` y `photon` y, si el mapa cambió, comprueba que `GET /static/manifest.json` informa el nuevo tamaño y checksum. Cada reconstrucción de Valhalla parte de cero y falla si no puede bajar la elevación, así que un fallo nunca deja un grafo plano ni uno viejo en uso.

**Privacidad**: ni el backend, ni Valhalla ni Photon registran el texto buscado ni las coordenadas de una ruta (lo comprueba `data-smoke.yml`). Si cambias el nivel de log de un motor, vuelve a comprobarlo.

## 6. Registros (logs)

- Caddy **no escribe registro de accesos**: la URI lleva bounding boxes, texto de búsqueda y la IP, y no registrarlas es más robusto que filtrarlas.
- El backend registra solo la ruta, sin la consulta. Los errores 5xx devuelven al cliente un mensaje genérico y un `error_id`; el detalle (que puede nombrar hosts o SQL) queda en el registro del servidor bajo ese identificador: pídelo al reportar un problema.
- Docker rota los registros (`json-file`, 10 MB × 3 archivos).

## 7. Mantenimiento periódico

- **Purga de caducados**: automática, cada cinco minutos y al arrancar; conserva 24 horas los caducados (ventana de topes). No requiere acción.
- **Copias de seguridad**: `pg_dump` del volumen `postgis_data` con el rol dueño. Los reportes caducan en horas; lo valioso son las cuentas.
- **Verificar un despliegue**: `GET /health`, `GET /source` (el `commit` debe ser el desplegado) y, en un entorno de pruebas, `scripts/e2e/audit_regression.py` (ver su cabecera).
- **Contrato de la API**: `contracts/openapi.json` se regenera con `make openapi`; el CI falla si está desactualizado.
