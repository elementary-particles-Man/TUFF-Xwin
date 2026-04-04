# TRADEMARKS

## 1. Purpose

This file records trademark-related notices and internal usage rules for names owned by third parties and referenced by this project.

---

## 2. Khronos / Vulkan Notice

Khronos and Vulkan are registered trademarks of The Khronos Group Inc.

This project may use a compute backend built on the Vulkan API where available.

Such use does not mean that The Khronos Group Inc. develops, distributes, operates, certifies, supports, endorses, or approves this project.

Unless separately and explicitly established under applicable Khronos requirements, this project does not claim:
- Khronos conformance
- Khronos compliance
- Khronos certification
- official Vulkan support status
- Khronos endorsement or approval

---

## 3. Public Wording Rules

### Allowed style
Use neutral descriptive language such as:
- "optional GPU compute backend"
- "experimental GPU compute acceleration"
- "compute backend built on the Vulkan API"
- "Vulkan-based compute path when available"

### Avoid
Do not use product-facing wording such as:
- "Vulkan compliant"
- "Vulkan conformant"
- "official Vulkan support"
- "full Vulkan implementation"
- "Khronos-certified"
- "approved by Khronos"

Do not use the Vulkan logo in relation to a shipped implementation unless the applicable Khronos requirements for such usage have actually been satisfied.

---

## 4. Non-Endorsement Statement

References to Khronos or Vulkan are for identification and attribution only.

No reference in this repository, product, manual, installer, dashboard, or release note should be interpreted as implying any partnership, sponsorship, endorsement, approval, certification, or operating responsibility by The Khronos Group Inc.

---

## 5. Responsibility Boundary

All responsibilities for this project—including design, implementation, behavior, maintenance, updates, support, runtime operation, failure handling, safety, and security—belong solely to this project and its own developers, distributors, integrators, and operators.

The Khronos Group Inc. is not responsible for any project-specific behavior or outcomes arising from the use of this project.

---

## 6. Internal Documentation Rule

When Vulkan must be mentioned in technical documentation:
- prefer short factual references;
- prefer linking to official Khronos materials rather than copying large specification passages;
- keep legal attribution separate from performance, compatibility, or feature marketing claims.

---

## 7. Release Review Checklist

Before release:

- [ ] No page, UI, README, manual, installer, dashboard, or release note claims Khronos conformance/compliance/support unless separately authorized.
- [ ] No Vulkan logo is used without confirming applicable rights.
- [ ] `THIRD_PARTY_NOTICES.md` accurately lists redistributed Khronos-originated components.
- [ ] The wording in manuals stays descriptive and non-promotional.
