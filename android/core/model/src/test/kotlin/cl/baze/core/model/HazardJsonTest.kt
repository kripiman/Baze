// SPDX-FileCopyrightText: 2026 Gabriel Piñones
// SPDX-License-Identifier: AGPL-3.0-or-later

package cl.baze.core.model

import kotlinx.serialization.json.Json
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The payloads below are what the backend (contracts/openapi.json) really sends. If the server or the
 * DTOs drift apart, these fail before an APK does.
 */
class HazardJsonTest {
    private val json = Json { ignoreUnknownKeys = true }

    private val payload = """
        {
          "id": "7c9e6679-7425-40de-944b-e07fc1f90ae7",
          "category": "road_closed",
          "hazard_type": "blocking",
          "status": "confirmed",
          "description": null,
          "upvotes": 3,
          "downvotes": 1,
          "location": {"type": "Point", "coordinates": [-70.65, -33.45]},
          "created_at": "2026-10-04T17:00:00.123456789Z",
          "expires_at": "2026-10-05T17:00:00Z"
        }
    """.trimIndent()

    @Test
    fun `decodes a hazard as the backend sends it`() {
        val hazard = json.decodeFromString<Hazard>(payload)

        assertEquals(HazardCategory.ROAD_CLOSED, hazard.category)
        assertEquals(HazardType.BLOCKING, hazard.hazardType)
        assertEquals(HazardStatus.CONFIRMED, hazard.status)
        assertNull(hazard.description)
        assertEquals(3, hazard.upvotes)
        assertEquals(-70.65, hazard.location.longitude, 0.0)
        assertEquals(-33.45, hazard.location.latitude, 0.0)
        assertNull(hazard.location.altitudeMeters)
        assertTrue(hazard.isConfirmedBlocking)
        assertTrue(hazard.expiresAt > hazard.createdAt)
    }

    @Test
    fun `a hazard without hazard_type derives it from the category`() {
        val withoutType = payload.replace("\"hazard_type\": \"blocking\",", "").replace("road_closed", "glass")

        val hazard = json.decodeFromString<Hazard>(withoutType)

        assertEquals(HazardType.WARNING, hazard.hazardType)
        assertFalse(hazard.isConfirmedBlocking)
    }

    @Test
    fun `every category maps to the type the server derives`() {
        val blocking = setOf(HazardCategory.ROAD_CLOSED, HazardCategory.CONSTRUCTION, HazardCategory.FLOOD)
        for (category in HazardCategory.entries) {
            val expected = if (category in blocking) HazardType.BLOCKING else HazardType.WARNING
            assertEquals(category.name, expected, category.hazardType)
        }
    }

    @Test
    fun `points keep their altitude and write plain GeoJSON`() {
        val flat = json.encodeToString(Coordinates.serializer(), Coordinates(-70.65, -33.45))
        val raised = json.encodeToString(Coordinates.serializer(), Coordinates(-70.65, -33.45, 540.0))

        assertEquals("""{"type":"Point","coordinates":[-70.65,-33.45]}""", flat)
        assertEquals("""{"type":"Point","coordinates":[-70.65,-33.45,540.0]}""", raised)
        assertEquals(540.0, json.decodeFromString(Coordinates.serializer(), raised).altitudeMeters)
    }

    @Test
    fun `a malformed point is rejected`() {
        assertThrows(IllegalArgumentException::class.java) {
            json.decodeFromString(Coordinates.serializer(), """{"type":"LineString","coordinates":[1.0,2.0]}""")
        }
        assertThrows(IllegalArgumentException::class.java) {
            json.decodeFromString(Coordinates.serializer(), """{"type":"Point","coordinates":[1.0]}""")
        }
    }

    @Test
    fun `bounding boxes use the query parameter names of the API`() {
        val box = BoundingBox(minLongitude = -70.7, minLatitude = -33.5, maxLongitude = -70.6, maxLatitude = -33.4)

        val encoded = json.encodeToString(BoundingBox.serializer(), box)

        assertEquals("""{"min_lon":-70.7,"min_lat":-33.5,"max_lon":-70.6,"max_lat":-33.4}""", encoded)
        assertTrue(box.contains(Coordinates(-70.65, -33.45)))
        assertFalse(box.contains(Coordinates(-71.0, -33.45)))
    }
}
