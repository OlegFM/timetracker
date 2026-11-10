# References

## Windows code signing (looked up 2026-11-10)

- [Microsoft: code signing options](https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/code-signing-options)
  - Store MSIX packages are re-signed by Microsoft: no SmartScreen warning, no certificate needed.
  - A Win32 MSI/EXE installer in the Store must be signed by the publisher.
  - Outside the Store, a new app warns on first download until it has a download history, however it is signed.
- [Free Store registration for individual developers](https://blogs.windows.com/windowsdeveloper/2025/09/10/free-developer-registration-for-individual-developers-on-microsoft-store/)
  (September 2025): sign in with a personal Microsoft account, no fee.
- [Azure Artifact Signing (Trusted Signing) quickstart](https://learn.microsoft.com/azure/trusted-signing/quickstart)
  and [pricing](https://azure.microsoft.com/pricing/details/artifact-signing/).
  - Individual developers only in the USA and Canada.
  - From USD 9.99/month.
- [SignPath Foundation terms](https://signpath.org/terms)
  - Free signing for open-source projects: OSI license, public repository, MFA.
  - The publisher shown is "SignPath Foundation".
- [Certum code signing (SimplySign, cloud key), reseller listing](https://sslpoint.com/code-signing-certificates)
  - Individuals allowed, roughly EUR 109–169 a year.
  - Prices unverified on certum.eu.
