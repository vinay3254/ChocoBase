cmake_minimum_required(VERSION 3.14)
project(chocobase VERSION 0.1.0 LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

add_library(chocobase INTERFACE)
target_include_directories(chocobase INTERFACE include)
