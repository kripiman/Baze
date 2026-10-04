// SPDX-FileCopyrightText: 2026 Gabriel Piñones
// SPDX-License-Identifier: AGPL-3.0-or-later

package cl.baze.core.network

import cl.baze.core.model.Account
import cl.baze.core.model.BoundingBox
import cl.baze.core.model.Coordinates
import cl.baze.core.model.Hazard
import cl.baze.core.model.Route
import kotlinx.coroutines.flow.Flow

/**
 * What the app needs from the Baze backend. Only types of `core:model` and the standard library
 * appear here, so no module has to put the HTTP stack (Ktor) on its own classpath to use it.
 */
interface BazeApi {
    suspend fun createAnonymousAccount(): Result<Account>

    suspend fun fetchHazards(bbox: BoundingBox): Result<List<Hazard>>

    suspend fun reportHazard(hazard: Hazard, token: String): Result<Hazard>

    suspend fun calculateRoute(origin: Coordinates, destination: Coordinates): Result<Route>

    fun observeHazardsStream(bbox: BoundingBox): Flow<Hazard>
}

/** The client the app uses. The implementation stays internal to this module. */
fun createBazeApi(baseUrl: String = DEFAULT_BASE_URL): BazeApi = KtorBazeApi(baseUrl)

// TODO(verify): reemplazar por un buildConfigField por entorno cuando el backend tenga dominio público.
const val DEFAULT_BASE_URL = "https://localhost"
