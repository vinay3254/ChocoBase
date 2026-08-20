cmake_minimum_required(VERSION 3.14)
project(cpp_quickstart VERSION 0.1.0 LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

include_directories(../../sdk/chocobase-cpp/include)

add_executable(quickstart main.cpp)
