# syntax=docker/dockerfile:1
# SPDX-FileCopyrightText: 2026 Gabriel Piñones
# SPDX-License-Identifier: AGPL-3.0-or-later
#
# The Photon geocoder (https://github.com/komoot/photon). The project publishes no container image, only the
# jar, so Baze builds its own: the release jar verified by checksum, on a pinned Java runtime.
#
# To upgrade Photon, change the version in the URL and the sha256 together (`sha256sum photon-X.Y.Z.jar` of the
# file on the GitHub release page), and the database dump you import must match its format (see data/README.md).
FROM eclipse-temurin:21.0.12.1_1-jre-noble@sha256:22138efd69393501fccd8176ae16b01791ed71ff801b28f0359415389b17c766

# ADD verifies the checksum itself: a different file under the same release URL fails the build.
ADD --checksum=sha256:a89707c0045e4807b2a1180e132e68e108d998709f48b6c94b98a6e281f571a5 --chmod=0644 \
    https://github.com/komoot/photon/releases/download/1.3.0/photon-1.3.0.jar /opt/photon/photon.jar

# The index lives in /photon/photon_data (a bind mount). Compose overrides the user with the one that built
# the data; this default is only for running the image by hand.
USER 10002:10002
WORKDIR /photon

# Embedded OpenSearch needs a heap of its own; override with JAVA_TOOL_OPTIONS for a bigger index.
ENV JAVA_TOOL_OPTIONS="-Xmx1g -Duser.home=/tmp"

EXPOSE 2322
ENTRYPOINT ["java", "-jar", "/opt/photon/photon.jar"]
# 0.0.0.0 inside the container only: the service is attached to the internal network and publishes no port.
CMD ["serve", "-data-dir", "/photon", "-listen-ip", "0.0.0.0", "-listen-port", "2322"]
