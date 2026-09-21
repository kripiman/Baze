<!--
SPDX-FileCopyrightText: 2026 Gabriel Piñones
SPDX-License-Identifier: AGPL-3.0-or-later
-->

# ADR 0006: Cuentas Anónimas, Votación Restringida y Rate Limiting

## Estado
Aceptado

## Contexto
Para fomentar la participación ciudadana en reportes y alertas viales, la fricción de entrada debe ser mínima: obligar al usuario a registrarse con correo electrónico, contraseñas o cuentas de Google/Apple antes de usar la app desalienta la adopción.
Por otro lado, permitir reportes y votos sin ninguna identidad facilitaría ataques de denegación de servicio, suplantación masiva y alteración maliciosa del estado de las rutas.

## Decisión
1. **Emisión de cuentas anónimas**:
   - En el primer inicio, la app móvil llama a `POST /api/v1/auth/anonymous`.
   - El backend genera un registro en la tabla `accounts` con un identificador único aleatorio (UUIDv4) y entrega al cliente un token criptográfico (JWT o bearer token opaco).
   - El cliente guarda este token en almacenamiento seguro del dispositivo (Keystore / EncryptedSharedPreferences).
2. **Restricción de voto único**:
   - La tabla relacional `hazard_votes` impone la restricción `UNIQUE(hazard_id, account_id)`.
   - Cada cuenta anónima solo puede votar una vez (positivo o negativo) por cada reporte específico.
3. **Control de Abuso y Rate Limiting**:
   - Se aplican middlewares de rate limit (ej. `tower_governor`) en los endpoints de mutación (`POST /api/v1/hazards`, `POST /api/v1/hazards/{id}/vote`, `POST /api/v1/auth/anonymous`).
   - El límite se evalúa tanto por la IP origen de la solicitud como por el token de la cuenta anónima.

## Consecuencias
### Positivas
- Experiencia de usuario inmediata: sin formularios de registro, verificación de emails ni contraseñas.
- Integridad de votos garantizada a nivel de base de datos sin comprometer la identidad personal del ciclista.
- Protección efectiva contra spam automatizado y ataques de denegación de servicio.

### Negativas / Riesgos
- Si el usuario borra los datos de la app o reinstala el sistema, se generará una nueva cuenta anónima (el historial previo no se vincula a una identidad recuperable).
