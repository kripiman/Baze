// SPDX-FileCopyrightText: 2026 Gabriel Piñones
// SPDX-License-Identifier: AGPL-3.0-or-later

package cl.baze.core.network

import cl.baze.core.model.Account
import cl.baze.core.model.BoundingBox
import cl.baze.core.model.Coordinates
import cl.baze.core.model.Hazard
import cl.baze.core.model.Route
import io.ktor.client.HttpClient
import io.ktor.client.call.body
import io.ktor.client.engine.cio.CIO
import io.ktor.client.plugins.contentnegotiation.ContentNegotiation
import io.ktor.client.plugins.logging.LogLevel
import io.ktor.client.plugins.logging.Logging
import io.ktor.client.request.get
import io.ktor.client.request.header
import io.ktor.client.request.parameter
import io.ktor.client.request.post
import io.ktor.client.request.setBody
import io.ktor.http.ContentType
import io.ktor.http.contentType
import io.ktor.serialization.kotlinx.json.json
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.emptyFlow
import kotlinx.serialization.json.Json

/**
 * Cliente de red para la API de Baze.
 *
 * TODO(verify): Integrar generador automático Gradle (ej. org.openapi.generator)
 * apuntando a `../../contracts/openapi.json` para generar interfaces y DTOs tipados.
 */
class BazeApiClient(
    private val baseUrl: String = "https://localhost",
    private val client: HttpClient = createDefaultHttpClient()
) {
    companion object {
        fun createDefaultHttpClient(): HttpClient {
            return HttpClient(CIO) {
                install(ContentNegotiation) {
                    json(Json {
                        ignoreUnknownKeys = true
                        prettyPrint = false
                        isLenient = true
                    })
                }
                install(Logging) {
                    level = LogLevel.INFO
                }
            }
        }
    }

    suspend fun createAnonymousAccount(): Result<Account> = runCatching {
        // TODO(verify): Implementar llamada POST /api/v1/auth/anonymous
        Account(id = "anon-dummy", token = "baze_anon_dummy")
    }

    suspend fun fetchHazards(bbox: BoundingBox): Result<List<Hazard>> = runCatching {
        // TODO(verify): Implementar llamada GET /api/v1/hazards con parámetros min_lon, min_lat, max_lon, max_lat
        emptyList()
    }

    suspend fun reportHazard(hazard: Hazard, token: String): Result<Hazard> = runCatching {
        // TODO(verify): Implementar llamada POST /api/v1/hazards con token en Authorization
        hazard
    }

    suspend fun calculateRoute(origin: Coordinates, destination: Coordinates): Result<Route> = runCatching {
        // TODO(verify): Implementar llamada POST /api/v1/routing/route
        Route(
            distanceMeters = 0.0,
            durationSeconds = 0.0,
            ascentMeters = 0.0,
            descentMeters = 0.0,
            points = listOf(origin, destination),
            maneuvers = emptyList(),
            nearbyHazards = emptyList()
        )
    }

    fun observeHazardsStream(bbox: BoundingBox): Flow<Hazard> {
        // TODO(verify): Implementar conexión SSE a /api/v1/realtime/sse?min_lon=... usando Ktor SSE
        return emptyFlow()
    }
}
