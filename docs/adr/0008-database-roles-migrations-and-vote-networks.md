<!--
SPDX-FileCopyrightText: 2026 Gabriel Piñones
SPDX-License-Identifier: AGPL-3.0-or-later
-->

# ADR 0008: Roles de Base de Datos, Migraciones y Etiquetas de Red de Voto

## Estado
Aceptado. Complementa [ADR-0001](0001-modular-monolith.md) y [ADR-0007](0007-identity-voting-and-abuse-limits.md).

## Contexto
Con la persistencia en PostgreSQL aparecen tres decisiones que antes no existían:

1. **Con qué privilegios habla el backend con la base de datos.** Si el backend usaba el usuario creado por la imagen de PostgreSQL, es superusuario: una ejecución remota de código en el backend (o una inyección SQL) equivale a control total del servidor de base de datos (`COPY ... PROGRAM`, `CREATE EXTENSION`, lectura de archivos).
2. **Quién aplica el esquema y cuándo.** Copiar `migrations/` a la imagen y que la aplicación tenga permiso de DDL ensancha ambas superficies.
3. **Cómo recordar qué redes ya votaron un reporte sin guardar direcciones IP**, que [ADR-0005](0005-no-telemetry-in-mvp.md) prohíbe.

## Decisión

### 1. Dos roles
- **Dueño del esquema** (`POSTGRES_USER`): se usa solo al inicializar el volumen y en el servicio `migrate`. Es el único contenedor que recibe sus credenciales (`MIGRATION_DATABASE_URL`).
- **`baze_app`**: lo crea `infra/postgres/init/01-roles.sh`; es `LOGIN` sin `SUPERUSER`, `CREATEDB`, `CREATEROLE`, `REPLICATION` ni `BYPASSRLS`. Solo tiene DML (`SELECT`, `INSERT`, `UPDATE`, `DELETE`) sobre las tablas, mediante `ALTER DEFAULT PRIVILEGES`. Límites: `CONNECTION LIMIT 25`, `statement_timeout = 5s`, `idle_in_transaction_session_timeout = 10s`, `lock_timeout = 3s`. El backend recibe `DATABASE_URL` con este rol y ninguna otra credencial.
- Al arrancar en producción, el backend **se niega a ejecutar** si su rol es superusuario o tiene alguno de esos atributos (`assert_least_privilege`). En desarrollo no se aplica esta comprobación, para poder usar el usuario local de PostgreSQL.
- El CI comprueba contra la pila real que `baze_app` no puede `CREATE TABLE`, `ALTER TABLE`, `DROP TABLE`, `CREATE EXTENSION`, `COPY ... TO PROGRAM` ni `CREATE ROLE`.

### 2. Migraciones
- Los archivos de `backend/migrations/` se **embeben** en el binario con `sqlx::migrate!`; no se copian a la imagen.
- `baze-server migrate` las aplica con el rol dueño y termina. En Compose es el servicio `migrate` (un solo uso); el backend depende de él con `service_completed_successfully`.
- Al arrancar, el backend verifica que la base esté en la versión de esquema que su binario espera (`assert_schema_current`) y falla si no, o si una migración aplicada fue alterada (suma de verificación distinta).
- **Una migración aplicada nunca se edita**: los cambios van en un archivo nuevo.

### 3. Etiqueta de red de voto (`hazard_votes.voter_net`)
Para respetar "solo cuenta el primer voto de cada red" tras un reinicio, cada boleta guarda una etiqueta de la red del votante:

```
voter_net = HMAC-SHA-256(clave, "baze/voter-net/v1\0" ‖ familia ‖ prefijo ‖ dirección de red)[0..16]
```

- La red es el `/24` (IPv4) o el `/64` (IPv6), con las direcciones IPv4 mapeadas en IPv6 reducidas a IPv4.
- **No es un hash simple**: los `/24` son solo 2²⁴ posibilidades y un hash sin clave se invierte probándolas todas. Con clave, una copia de la base de datos no permite comprobar si un voto vino de una red dada.
- La clave es `JWT_SECRET` con su propia etiqueta de dominio, distinta de la de los tokens; no hay un secreto adicional que configurar. `JWT_SECRET_PREVIOUS` no interviene.
- Truncada a 128 bits y guardada como `BYTEA` de 16 bytes (`CHECK (octet_length(voter_net) = 16)`). Ninguna columna de la base de datos puede contener una dirección.
- **Consecuencia de rotar `JWT_SECRET`**: todas las etiquetas cambian, así que una red puede volver a votar una vez en los reportes que sigan vivos (como mucho, un TTL de reportes). Es un coste acotado y aceptado; `docs/operations.md` lo recuerda en el procedimiento de rotación.

### 4. Integridad de los votos bajo concurrencia
- Votar es una transacción que primero bloquea la fila del reporte (`SELECT ... FOR UPDATE`): todos los votos de un mismo reporte se serializan, el recuento ve cada boleta anterior y la regla del primer voto por red no puede perder una carrera.
- Red de seguridad: índice único parcial `(hazard_id, voter_net) WHERE counts`; aunque la aplicación fallara, la base de datos no admite dos boletas contadas de la misma red en un reporte.
- Los contadores `upvotes` y `downvotes` de `hazards` son derivados de las boletas y se recalculan dentro de la misma transacción; una `CHECK` impide valores negativos.

### 5. Purga
Un job cada cinco minutos (y al arrancar) borra en lotes de 1000 los reportes caducados hace más de 24 horas; las boletas caen en cascada. Las lecturas ignoran los caducados desde el instante en que vencen, de modo que la purga solo recupera espacio. La gracia de 24 horas es la ventana de los topes diarios de [ADR-0007](0007-identity-voting-and-abuse-limits.md).

## Consecuencias

### Positivas
- Comprometer el proceso del backend ya no entrega el servidor de base de datos.
- Las migraciones y las credenciales con permiso de DDL viven en un contenedor que termina y no está expuesto.
- La deduplicación de votos sobrevive a reinicios sin guardar direcciones IP.

### Negativas / Riesgos
- Rotar `JWT_SECRET` reabre un voto por red en los reportes vivos (acotado por el TTL).
- Un volumen de PostgreSQL creado antes de este ADR no ejecuta los scripts de `docker-entrypoint-initdb.d`: hay que crear `baze_app` a mano (ver `docs/operations.md`).
- Las consultas usan `sqlx::query` en tiempo de ejecución (no `query!`), de modo que un error de SQL aparece al ejecutar y no al compilar; los tests con base de datos del CI (`sqlx::test`, con PostGIS real) son el seguro contra ello.
