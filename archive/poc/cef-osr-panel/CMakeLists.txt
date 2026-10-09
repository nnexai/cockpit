cmake_minimum_required(VERSION 3.21)

project(cef_osr_panel VERSION 0.1.0 LANGUAGES C CXX)

set(CEF_ARTIFACT "cef_binary_150.0.20+ga832838+chromium-150.0.7871.253_linux64_minimal.tar.bz2"
    CACHE STRING "Pinned official CEF Linux64 minimal artifact")
set(CEF_ARTIFACT_SHA1 "5dfbe1331a17a921c57a4ebbf334646d55c2e1b0"
    CACHE STRING "SHA1 for the pinned CEF artifact")
set(CEF_DOWNLOAD_BASE "https://cef-builds.spotifycdn.com/"
    CACHE STRING "Official CEF download base URL")

if(NOT CEF_ROOT AND DEFINED ENV{CEF_ROOT})
  set(CEF_ROOT "$ENV{CEF_ROOT}" CACHE PATH "Official CEF binary distribution root")
else()
  set(CEF_ROOT "" CACHE PATH "Official CEF binary distribution root")
endif()

if(NOT CEF_ROOT)
  message(FATAL_ERROR
    "CEF_ROOT is required. Run scripts/fetch-cef.sh or configure with "
    "-DCEF_ROOT=/absolute/path/to/official/CEF distribution.")
endif()

get_filename_component(CEF_ROOT "${CEF_ROOT}" ABSOLUTE)
if(NOT EXISTS "${CEF_ROOT}/include/cef_app.h")
  message(FATAL_ERROR "CEF_ROOT does not contain include/cef_app.h: ${CEF_ROOT}")
endif()
if(NOT EXISTS "${CEF_ROOT}/Release/libcef.so")
  message(FATAL_ERROR "CEF_ROOT does not contain Release/libcef.so: ${CEF_ROOT}")
endif()

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)
set(CMAKE_POSITION_INDEPENDENT_CODE ON)

find_package(Threads REQUIRED)
find_package(X11 REQUIRED)

set(CMAKE_MODULE_PATH ${CMAKE_MODULE_PATH} "${CEF_ROOT}/cmake")
find_package(CEF REQUIRED)
SET_CEF_TARGET_OUT_DIR()
file(MAKE_DIRECTORY "${CEF_TARGET_OUT_DIR}")
foreach(CEF_BINARY_FILE IN LISTS CEF_BINARY_FILES)
  set(CEF_BINARY_SOURCE "${CEF_BINARY_DIR_RELEASE}/${CEF_BINARY_FILE}")
  set(CEF_BINARY_DESTINATION "${CEF_TARGET_OUT_DIR}/${CEF_BINARY_FILE}")
  if(EXISTS "${CEF_BINARY_SOURCE}" AND NOT EXISTS "${CEF_BINARY_DESTINATION}" AND
     NOT IS_SYMLINK "${CEF_BINARY_DESTINATION}")
    file(CREATE_LINK "${CEF_BINARY_SOURCE}" "${CEF_BINARY_DESTINATION}" SYMBOLIC)
  endif()
endforeach()
add_subdirectory("${CEF_LIBCEF_DLL_WRAPPER_PATH}" "${CMAKE_CURRENT_BINARY_DIR}/libcef_dll_wrapper")

add_executable(cef-osr-panel src/main.cpp)
SET_EXECUTABLE_TARGET_PROPERTIES(cef-osr-panel)
target_include_directories(cef-osr-panel PRIVATE
  "${CEF_ROOT}"
  "${CEF_ROOT}/include"
  ${X11_INCLUDE_DIR}
)
target_compile_definitions(cef-osr-panel PRIVATE
  CEF_OSR_PANEL_DEFAULT_WIDTH=1024
  CEF_OSR_PANEL_DEFAULT_HEIGHT=720
)
ADD_LOGICAL_TARGET("libcef_lib" "${CEF_LIB_DEBUG}" "${CEF_LIB_RELEASE}")
target_link_libraries(cef-osr-panel PRIVATE
  libcef_lib
  libcef_dll_wrapper
  ${CEF_STANDARD_LIBS}
  Threads::Threads
  ${CMAKE_DL_LIBS}
)
set_target_properties(cef-osr-panel PROPERTIES
  BUILD_RPATH "$ORIGIN;${CEF_BINARY_DIR_RELEASE}"
  INSTALL_RPATH "$ORIGIN;${CEF_BINARY_DIR_RELEASE}"
)

# CEF expects libcef and the helper resources beside the executable. The binary
# files are symlinked above for this Linux POC to avoid copying the 1+ GiB
# libcef.so during every development build.
add_custom_command(TARGET cef-osr-panel POST_BUILD
  COMMAND ${CMAKE_COMMAND} -E copy_directory
    "${CEF_RESOURCE_DIR}"
    "$<TARGET_FILE_DIR:cef-osr-panel>"
  COMMENT "Staging the official CEF runtime resources beside cef-osr-panel"
)

install(TARGETS cef-osr-panel RUNTIME DESTINATION bin)
