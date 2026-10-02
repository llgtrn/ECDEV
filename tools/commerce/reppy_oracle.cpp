// ECDEV test-only harness for the locked MIT-licensed Reppy parser.
// Donor sources remain immutable, outside ECDEV's production dependency graph.
#include <iostream>
#include <iomanip>
#include <sstream>
#include <string>
#include "robots.h"
static std::string unhex(const std::string& hex) {
    std::string out;
    for (size_t i=0; i<hex.size(); i+=2) out.push_back(static_cast<char>(std::stoul(hex.substr(i,2),nullptr,16)));
    return out;
}
int main() {
    std::string content,url,agent;
    while(std::cin >> content >> url >> agent) {
        try {
            Rep::Robots robots(unhex(content),"https://shop.example/robots.txt");
            auto selected=unhex(agent);
            std::cout << robots.allowed(unhex(url),selected) << " " << std::setprecision(9) << robots.agent(selected).delay() << "\n";
        } catch(const std::exception& error) { std::cerr << error.what() << "\n"; return 1; }
    }
}
